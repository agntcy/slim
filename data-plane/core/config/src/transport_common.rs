// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Transport-agnostic connector / proxy / header helpers shared between the
//! gRPC and WebSocket clients. Both transports build their hyper connector
//! chain the same way (HttpConnector → optional Tunnel → optional TLS), so
//! these helpers live on `ClientConfig` once and are called by both.

use std::collections::HashMap;
use std::str::FromStr;

use base64::prelude::*;
use http::Uri;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::connect::proxy::Tunnel;
use hyper_util::client::proxy::matcher::Intercept;
use rustls_pki_types::ServerName;
use tracing::warn;

use crate::client::ClientConfig;
use crate::errors::ConfigError;
use crate::grpc::proxy::ProxyConfig;
use crate::tls::common::RustlsConfigLoader;

/// ALPN selection for `build_https_connector`. gRPC needs HTTP/2; WebSocket's
/// HTTP/1.1 upgrade needs HTTP/1.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Alpn {
    Http1,
    Http2,
}

/// Build an HTTPS connector wrapping `inner`, with optional SNI override.
/// Shared between gRPC (HTTP/2) and WebSocket (HTTP/1) — only the ALPN
/// negotiation differs.
pub(crate) fn build_https_connector<S>(
    inner: S,
    tls: rustls::ClientConfig,
    server_name: Option<String>,
    alpn: Alpn,
) -> HttpsConnector<S>
where
    S: tower::Service<Uri>,
{
    let mut builder = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_or_http();
    if let Some(name) = server_name {
        builder =
            builder.with_server_name_resolver(move |_: &_| ServerName::try_from(name.clone()));
    }
    match alpn {
        Alpn::Http1 => builder.enable_http1().wrap_connector(inner),
        Alpn::Http2 => builder.enable_http2().wrap_connector(inner),
    }
}

/// Output of [`ClientConfig::build_proxy_tunnel`]. The two variants exist
/// because an HTTPS proxy needs its own TLS to be established *to* the proxy
/// before the CONNECT tunnel is opened.
pub(crate) enum ProxyTunnel {
    Http(Tunnel<HttpConnector>),
    Https(Tunnel<HttpsConnector<HttpConnector>>),
}

impl ClientConfig {
    /// Build a `HttpConnector` honoring the client's connect timeout and
    /// keepalive settings. `enforce_http(false)` lets the connector be used
    /// with `https://` URIs once wrapped by an `HttpsConnector`.
    pub(crate) fn create_http_connector(&self) -> Result<HttpConnector, ConfigError> {
        let mut http = HttpConnector::new();

        // NOTE(msardara): we might want to make these configurable as well.
        http.enforce_http(false);
        http.set_nodelay(false);

        match self.connect_timeout.as_secs() {
            0 => http.set_connect_timeout(None),
            _ => http.set_connect_timeout(Some(self.connect_timeout.into())),
        }

        if let Some(keepalive) = &self.keepalive {
            http.set_keepalive(Some(keepalive.tcp_keepalive.into()));
        }

        Ok(http)
    }

    pub(crate) fn parse_headers(&self) -> Result<HeaderMap, ConfigError> {
        Self::parse_header_map(&self.headers)
    }

    pub(crate) fn parse_header_map(
        headers: &HashMap<String, String>,
    ) -> Result<HeaderMap, ConfigError> {
        let mut header_map = HeaderMap::new();
        for (key, value) in headers {
            let header_name = HeaderName::from_str(key)?;
            let header_value = HeaderValue::from_str(value)?;
            header_map.insert(header_name, header_value);
        }
        Ok(header_map)
    }

    pub(crate) fn parse_proxy_headers(
        &self,
        headers: &HashMap<String, String>,
    ) -> Result<HeaderMap, ConfigError> {
        Self::parse_header_map(headers)
    }

    pub(crate) fn create_proxy_auth_header(
        username: &str,
        password: &str,
    ) -> Result<HeaderValue, ConfigError> {
        let auth_value = BASE64_STANDARD.encode(format!("{}:{}", username, password));
        Ok(HeaderValue::from_str(&format!("Basic {}", auth_value))?)
    }

    pub(crate) fn apply_tunnel_config<T>(
        &self,
        mut tunnel: Tunnel<T>,
        proxy_config: &ProxyConfig,
        warn_insecure: bool,
    ) -> Result<Tunnel<T>, ConfigError> {
        if let (Some(username), Some(password)) = (&proxy_config.username, &proxy_config.password) {
            if warn_insecure {
                self.warn_insecure_auth();
            }
            let auth_header = Self::create_proxy_auth_header(username, password)?;
            tunnel = tunnel.with_auth(auth_header);
        }

        if !proxy_config.headers.is_empty() {
            let proxy_headers = self.parse_proxy_headers(&proxy_config.headers)?;
            tunnel = tunnel.with_headers(proxy_headers);
        }

        Ok(tunnel)
    }

    pub(crate) async fn load_tls_config(
        &self,
    ) -> Result<Option<rustls::ClientConfig>, ConfigError> {
        Ok(self.tls_setting.load_rustls_config().await?)
    }

    pub(crate) fn warn_insecure_auth(&self) {
        if self.tls_setting.insecure {
            warn!("Auth is enabled without TLS. This is not recommended.");
        }
    }

    /// Build a CONNECT tunnel to a proxy, with auth/headers applied via
    /// [`Self::apply_tunnel_config`]. Returns `ProxyTunnel::Https` if the
    /// proxy URL is `https://` (needing TLS to the proxy itself with the
    /// given `proxy_alpn`), `ProxyTunnel::Http` otherwise.
    ///
    /// `proxy_alpn` only affects the TLS-to-proxy leg; the inner tunneled
    /// connection is plain bytes and the caller layers its own TLS + ALPN on
    /// top via [`build_https_connector`].
    pub(crate) async fn build_proxy_tunnel(
        &self,
        intercept: Intercept,
        http_connector: HttpConnector,
        proxy_alpn: Alpn,
    ) -> Result<ProxyTunnel, ConfigError> {
        let proxy_uri = intercept.uri().clone();
        tracing::info!(%proxy_uri, "Creating proxy tunnel");

        if proxy_uri.scheme_str() == Some("https") {
            let proxy_tls = self
                .proxy
                .tls_setting
                .load_rustls_config()
                .await?
                .ok_or(ConfigError::ProxyTlsMissing)?;
            let https_to_proxy = build_https_connector(http_connector, proxy_tls, None, proxy_alpn);
            let tunnel = Tunnel::new(proxy_uri, https_to_proxy);
            let configured = self.apply_tunnel_config(tunnel, &self.proxy, false)?;
            Ok(ProxyTunnel::Https(configured))
        } else {
            let tunnel = Tunnel::new(proxy_uri, http_connector);
            let configured = self.apply_tunnel_config(tunnel, &self.proxy, true)?;
            Ok(ProxyTunnel::Http(configured))
        }
    }
}
