// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Transport-agnostic connector / proxy / header helpers shared between the
//! gRPC and WebSocket clients. Both transports build their hyper connector
//! chain the same way (HttpConnector → optional Tunnel → optional TLS), so
//! these helpers live on `ClientConfig` once and are called by both.

use std::collections::HashMap;
use std::str::FromStr;

use base64::prelude::*;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::connect::proxy::Tunnel;
use tracing::warn;

use crate::client::ClientConfig;
use crate::errors::ConfigError;
use crate::grpc::proxy::ProxyConfig;
use crate::tls::common::RustlsConfigLoader;

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
}
