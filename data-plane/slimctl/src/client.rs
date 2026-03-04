// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result, bail};
use tonic::codegen::{Body, Bytes, StdError};

use slim_config::auth::basic::Config as BasicAuthConfig;
use slim_config::grpc::client::{AuthenticationConfig, BackoffConfig, ClientConfig};
use slim_config::tls::client::TlsClientConfig;

use crate::config::ResolvedOpts;
use crate::proto::controller::proto::v1::controller_service_client::ControllerServiceClient;
use crate::proto::controlplane::proto::v1::control_plane_service_client::ControlPlaneServiceClient;

/// Build a `ClientConfig` from the resolved connection options.
///
/// If the server address already includes a scheme (`http://` / `https://`) it is
/// used as-is after validating that it does not conflict with the TLS flags.
/// Otherwise the scheme is derived from `--tls-insecure`.
fn build_client_config(opts: &ResolvedOpts) -> Result<ClientConfig> {
    if opts.tls_cert_file.is_empty() ^ opts.tls_key_file.is_empty() {
        bail!("both tls-cert-file and tls-key-file must be specified together");
    }

    let (endpoint, use_tls) = if opts.server.starts_with("http://") {
        if opts.tls_insecure_skip_verify {
            bail!("--tls-insecure-skip-verify has no effect with an http:// server address");
        }
        (opts.server.clone(), false)
    } else if opts.server.starts_with("https://") {
        if opts.tls_insecure {
            bail!("--tls-insecure conflicts with an https:// server address");
        }
        (opts.server.clone(), true)
    } else {
        let scheme = if opts.tls_insecure { "http" } else { "https" };
        (format!("{}://{}", scheme, opts.server), !opts.tls_insecure)
    };

    let tls = if !use_tls {
        TlsClientConfig::insecure()
    } else if opts.tls_insecure_skip_verify {
        TlsClientConfig::new().with_insecure_skip_verify(true)
    } else {
        let mut cfg = TlsClientConfig::default();
        if !opts.tls_ca_file.is_empty() {
            cfg = cfg.with_ca_file(&opts.tls_ca_file);
        }
        if !opts.tls_cert_file.is_empty() {
            cfg = cfg.with_cert_and_key_file(&opts.tls_cert_file, &opts.tls_key_file);
        }
        cfg
    };

    let auth = if opts.basic_auth_creds.is_empty() {
        AuthenticationConfig::None
    } else {
        let (user, pass) = opts
            .basic_auth_creds
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("basic-auth-creds must be 'username:password'"))?;
        AuthenticationConfig::Basic(BasicAuthConfig::new(user, pass))
    };

    Ok(ClientConfig::with_endpoint(&endpoint)
        .with_tls_setting(tls)
        .with_connect_timeout(opts.timeout)
        .with_request_timeout(opts.timeout)
        .with_backoff(BackoffConfig::new_fixed_interval(
            std::time::Duration::from_millis(0),
            0,
        ))
        .with_auth(auth))
}

/// Create a `ControlPlaneServiceClient` with TLS and auth configured from `opts`.
pub async fn get_control_plane_client(
    opts: &ResolvedOpts,
) -> Result<
    ControlPlaneServiceClient<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<StdError> + Send,
            ResponseBody: Body<Data = Bytes, Error: Into<StdError> + Send> + Send + 'static,
            Future: Send,
        > + Clone
        + Send
        + 'static,
    >,
> {
    let channel = build_client_config(opts)?
        .to_channel()
        .await
        .context("failed to connect to server")?;
    Ok(ControlPlaneServiceClient::new(channel))
}

/// Create a `ControllerServiceClient` with TLS and auth configured from `opts`.
pub async fn get_controller_client(
    opts: &ResolvedOpts,
) -> Result<
    ControllerServiceClient<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<StdError> + Send,
            ResponseBody: Body<Data = Bytes, Error: Into<StdError> + Send> + Send + 'static,
            Future: Send,
        > + Clone
        + Send
        + 'static,
    >,
> {
    let channel = build_client_config(opts)?
        .to_channel()
        .await
        .context("failed to connect to server")?;
    Ok(ControllerServiceClient::new(channel))
}

/// Apply a request timeout to a tonic Request.
pub fn with_timeout<T>(req: T, opts: &ResolvedOpts) -> tonic::Request<T> {
    let mut request = tonic::Request::new(req);
    request.set_timeout(opts.timeout);
    request
}

/// Call a unary RPC method, wrapping the request with a per-request timeout
/// and unwrapping the inner response value.
///
/// Usage: `rpc!(client, method_name, request, opts)`
#[macro_export]
macro_rules! rpc {
    ($client:expr, $method:ident, $req:expr, $opts:expr) => {
        $client
            .$method($crate::client::with_timeout($req, $opts))
            .await?
            .into_inner()
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn base_opts() -> ResolvedOpts {
        ResolvedOpts {
            server: "localhost:50051".to_string(),
            timeout: Duration::from_secs(15),
            tls_insecure: true,
            tls_insecure_skip_verify: false,
            tls_ca_file: String::new(),
            tls_cert_file: String::new(),
            tls_key_file: String::new(),
            basic_auth_creds: String::new(),
        }
    }

    // ── build_client_config validation ──────────────────────────────────────

    #[test]
    fn cert_without_key_fails() {
        let mut opts = base_opts();
        opts.tls_cert_file = "/path/to/cert.pem".to_string();
        assert!(build_client_config(&opts).is_err());
    }

    #[test]
    fn key_without_cert_fails() {
        let mut opts = base_opts();
        opts.tls_key_file = "/path/to/key.pem".to_string();
        assert!(build_client_config(&opts).is_err());
    }

    #[test]
    fn http_server_with_skip_verify_fails() {
        let mut opts = base_opts();
        opts.server = "http://localhost:50051".to_string();
        opts.tls_insecure_skip_verify = true;
        assert!(build_client_config(&opts).is_err());
    }

    #[test]
    fn https_server_with_tls_insecure_fails() {
        let mut opts = base_opts();
        opts.server = "https://localhost:50051".to_string();
        opts.tls_insecure = true;
        assert!(build_client_config(&opts).is_err());
    }

    #[test]
    fn basic_auth_without_colon_fails() {
        let mut opts = base_opts();
        opts.basic_auth_creds = "usernameonly".to_string();
        assert!(build_client_config(&opts).is_err());
    }

    #[test]
    fn http_explicit_server_succeeds() {
        let mut opts = base_opts();
        opts.server = "http://localhost:50051".to_string();
        opts.tls_insecure = false;
        assert!(build_client_config(&opts).is_ok());
    }

    #[test]
    fn https_explicit_server_succeeds() {
        let mut opts = base_opts();
        opts.server = "https://localhost:50051".to_string();
        opts.tls_insecure = false;
        assert!(build_client_config(&opts).is_ok());
    }

    #[test]
    fn bare_server_with_tls_insecure_succeeds() {
        // base_opts has tls_insecure = true
        assert!(build_client_config(&base_opts()).is_ok());
    }

    #[test]
    fn bare_server_with_tls_enabled_succeeds() {
        let mut opts = base_opts();
        opts.tls_insecure = false;
        assert!(build_client_config(&opts).is_ok());
    }

    #[test]
    fn valid_basic_auth_with_colon_succeeds() {
        let mut opts = base_opts();
        opts.basic_auth_creds = "user:password".to_string();
        assert!(build_client_config(&opts).is_ok());
    }

    #[test]
    fn cert_and_key_together_succeeds() {
        let mut opts = base_opts();
        opts.tls_insecure = false;
        opts.tls_cert_file = "/path/to/cert.pem".to_string();
        opts.tls_key_file = "/path/to/key.pem".to_string();
        assert!(build_client_config(&opts).is_ok());
    }

    // ── with_timeout ────────────────────────────────────────────────────────

    #[test]
    fn with_timeout_wraps_request() {
        let opts = base_opts();
        let req = with_timeout("hello", &opts);
        assert_eq!(req.into_inner(), "hello");
    }

    #[test]
    fn bare_server_skip_verify_succeeds() {
        let mut opts = base_opts();
        opts.tls_insecure = false;
        opts.tls_insecure_skip_verify = true;
        assert!(build_client_config(&opts).is_ok());
    }

    #[test]
    fn bare_server_with_ca_file_succeeds() {
        let mut opts = base_opts();
        opts.tls_insecure = false;
        opts.tls_ca_file = "/path/ca.pem".to_string();
        assert!(build_client_config(&opts).is_ok());
    }
}
