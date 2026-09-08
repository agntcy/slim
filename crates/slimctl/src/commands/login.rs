// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::Args;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use oauth2::{CsrfToken, PkceCodeChallenge};
use reqwest::Client;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::Instant;
use url::Url;

// Asymmetric algorithms only; HS* are excluded because a public client has no
// shared secret and accepting them enables algorithm-confusion attacks.
const SAFE_ALGS: &[Algorithm] = &[
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::ES256,
    Algorithm::ES384,
];

const WELL_KNOWN_SUFFIX: &str = "/.well-known/openid-configuration";
// RFC 8252 section 8.3: native app redirects must use the loopback interface.
const LOOPBACK_HOSTS: &[&str] = &["localhost", "127.0.0.1", "::1"];

#[derive(Args)]
pub struct LoginArgs {
    /// OIDC client ID
    #[arg(long)]
    client_id: String,

    /// OIDC Connect discovery URL (the .well-known/openid-configuration endpoint)
    #[arg(long)]
    discovery_uri: String,

    /// Loopback redirect URI; must be registered with the provider exactly as given
    #[arg(long, default_value = "http://127.0.0.1:8250/callback")]
    redirect_uri: String,

    /// Seconds to wait for the browser redirect
    #[arg(long = "callback-timeout", default_value = "300", value_parser = clap::value_parser!(u64).range(1..))]
    callback_timeout: u64,

    /// Create an app identity: bind the issued token to a freshly generated MLS
    /// signing key using DPoP (RFC 9449), and write both to this store.
    ///
    /// Lets a SLIM app adopt your SSO identity — it installs the saved key as
    /// its MLS signing identity, and peers re-hash that key against the token's
    /// `cnf.jkt`. Requires an IdP with DPoP enabled (officially supported from
    /// Keycloak 26.4; experimental from 23.0 with `--features=dpop`).
    ///
    /// Nothing else is written: no bearer token file, and no change to the
    /// connection config. Point one app at the store with `credentials_file` in
    /// its OIDC identity config, or with `SLIM_CREDENTIALS_FILE`.
    ///
    /// A store holds one MLS key, and that key *is* the app's identity, so apps
    /// sharing a store cannot be removed from a group individually — give each
    /// app its own. The path is mandatory for the same reason: a DPoP-bound
    /// login in the default store would be unusable, since the identity path has
    /// no default store and the transport path cannot refresh a bound token.
    ///
    /// Omit for a plain bearer login, which goes to `~/.slimctl/credentials.yaml`
    /// for slimctl's own use. Node transport auth does not read that file; it
    /// needs `client_secret`, `refresh_token_file` or `refresh_token`.
    #[arg(long)]
    dpop_credentials_file: Option<String>,
}

struct ProviderMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    code_challenge_methods_supported: Vec<String>,
    response_types_supported: Vec<String>,
    id_token_signing_alg_values_supported: Vec<String>,
}

async fn fetch_metadata(client: &Client, discovery_url: &str) -> Result<ProviderMetadata> {
    check_scheme(discovery_url).context("discovery URL")?;

    let doc: Value = client
        .get(discovery_url)
        .send()
        .await
        .context("fetching discovery document")?
        .error_for_status()?
        .json()
        .await
        .context("parsing discovery document")?;

    let str_field = |key: &str| -> Result<String> {
        doc.get(key)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .with_context(|| format!("discovery document missing '{key}'"))
    };

    let str_array = |key: &str| -> Vec<String> {
        doc.get(key)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };

    // OIDC Core 4.3: the issuer in the document must match the URL it was served
    // from, preventing a misconfigured or tampered document from redirecting us to
    // a different issuer whose tokens we would then accept.
    let raw_issuer = str_field("issuer")?;
    let issuer = raw_issuer.trim_end_matches('/');
    if let Some(stripped) = discovery_url.strip_suffix(WELL_KNOWN_SUFFIX) {
        let expected = stripped.trim_end_matches('/');
        if issuer != expected {
            bail!("issuer mismatch: document claims {issuer:?} but was served from {expected:?}");
        }
    }

    let authorization_endpoint = str_field("authorization_endpoint")?;
    let token_endpoint = str_field("token_endpoint")?;
    let jwks_uri = str_field("jwks_uri")?;
    for (name, url) in [
        ("authorization_endpoint", &authorization_endpoint),
        ("token_endpoint", &token_endpoint),
        ("jwks_uri", &jwks_uri),
    ] {
        check_scheme(url).with_context(|| name.to_string())?;
    }

    Ok(ProviderMetadata {
        issuer: issuer.to_owned(),
        authorization_endpoint,
        token_endpoint,
        jwks_uri,
        code_challenge_methods_supported: str_array("code_challenge_methods_supported"),
        response_types_supported: str_array("response_types_supported"),
        id_token_signing_alg_values_supported: str_array("id_token_signing_alg_values_supported"),
    })
}

fn check_scheme(url: &str) -> Result<()> {
    if !url.to_ascii_lowercase().starts_with("https://") {
        bail!("must be an https URL, got {url:?}");
    }
    Ok(())
}

async fn bind_callback_listener(host: &str, port: u16) -> Result<TcpListener> {
    TcpListener::bind((host, port))
        .await
        .with_context(|| format!("binding {host}:{port}"))
}

async fn wait_for_callback(
    listener: TcpListener,
    path: &str,
    timeout_secs: u64,
) -> Result<(String, String)> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        let (mut stream, _) = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .context("timed out waiting for browser redirect — if your browser showed an error, check your --client-id and --discovery-uri")?
            .context("accepting connection")?;

        let mut buf = [0u8; 4096];
        // Apply the same deadline to the read so a silent speculative connection
        // cannot hold the loop until the outer timeout expires.
        let n = match tokio::time::timeout_at(deadline, stream.read(&mut buf)).await {
            Ok(Ok(n)) => n,
            _ => continue,
        };
        let text = String::from_utf8_lossy(&buf[..n]);
        let first_line = text.lines().next().unwrap_or("");

        let raw_path = first_line.split_whitespace().nth(1).unwrap_or("/");
        let parsed = Url::parse(&format!("http://localhost{raw_path}"))
            .unwrap_or_else(|_| Url::parse("http://localhost/").unwrap());

        if parsed.path() != path {
            // Browsers make spurious requests (e.g. /favicon.ico); ignore them and keep waiting.
            stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .ok();
            continue;
        }

        let mut code = None;
        let mut state = None;
        let mut error = None;
        let mut error_description = None;
        for (k, v) in parsed.query_pairs() {
            match k.as_ref() {
                "code" => code = Some(v.into_owned()),
                "state" => state = Some(v.into_owned()),
                "error" => error = Some(v.into_owned()),
                "error_description" => error_description = Some(v.into_owned()),
                _ => {}
            }
        }

        if let Some(err) = error {
            let desc = error_description.as_deref().unwrap_or("");
            let err_escaped = err
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let body = format!("<html><body>Sign-in failed: {err_escaped}</body></html>");
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .ok();
            bail!("provider error: {err} - {desc}");
        }

        let success_body =
            "<html><body>Authentication complete. You may close this tab.</body></html>";
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    success_body.len(),
                    success_body
                )
                .as_bytes(),
            )
            .await
            .ok();

        return Ok((
            code.context("redirect missing authorization code")?,
            state.context("redirect missing state")?,
        ));
    }
}

async fn validate_id_token(
    client: &Client,
    meta: &ProviderMetadata,
    id_token: &str,
    client_id: &str,
    nonce: &str,
) -> Result<serde_json::Map<String, Value>> {
    let jwks: JwkSet = client
        .get(&meta.jwks_uri)
        .send()
        .await
        .context("fetching JWKS")?
        .error_for_status()
        .context("JWKS endpoint returned an error")?
        .json()
        .await
        .context("parsing JWKS")?;

    let header = decode_header(id_token).context("decoding ID token header")?;

    if !SAFE_ALGS.contains(&header.alg) {
        bail!(
            "ID token uses {:?}; only asymmetric algorithms are accepted",
            header.alg
        );
    }

    let jwk = match &header.kid {
        Some(kid) => jwks
            .keys
            .iter()
            .find(|k| k.common.key_id.as_deref() == Some(kid.as_str()))
            .with_context(|| format!("no JWK for kid={kid:?}"))?,
        None => match jwks.keys.as_slice() {
            [single] => single,
            _ => bail!("no kid in ID token and JWKS has {} keys", jwks.keys.len()),
        },
    };

    let key = DecodingKey::from_jwk(jwk).context("building decoding key")?;
    let mut validation = Validation::new(header.alg);
    validation.set_audience(&[client_id]);
    validation.set_issuer(&[&meta.issuer]);
    validation.leeway = 60;
    validation
        .required_spec_claims
        .extend(["sub", "iat"].map(str::to_owned));
    let claims = decode::<serde_json::Map<String, Value>>(id_token, &key, &validation)
        .context("validating ID token")?
        .claims;

    if claims.get("nonce").and_then(|v| v.as_str()) != Some(nonce) {
        bail!("ID token nonce mismatch");
    }

    // OIDC Core 3.1.3.7: when azp is present it must identify this client.
    if let Some(azp) = claims.get("azp").and_then(|v| v.as_str())
        && azp != client_id
    {
        bail!("ID token azp claim does not match client_id");
    }

    Ok(claims)
}

// Constant-time comparison to resist timing oracle attacks on the CSRF state token.
fn ct_str_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && {
        let diff = a
            .bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y));
        diff == 0
    }
}

pub async fn run(args: &LoginArgs, config_file: Option<&str>, server: Option<&str>) -> Result<()> {
    let http = Client::builder()
        .user_agent("slimctl")
        .timeout(Duration::from_secs(30))
        .build()?;

    let meta = fetch_metadata(&http, &args.discovery_uri).await?;

    if !meta.response_types_supported.iter().any(|s| s == "code") {
        bail!("provider does not support authorization code flow");
    }
    if !meta
        .code_challenge_methods_supported
        .iter()
        .any(|s| s == "S256")
    {
        bail!("provider does not support PKCE S256");
    }
    if !meta.id_token_signing_alg_values_supported.is_empty() {
        let acceptable = meta
            .id_token_signing_alg_values_supported
            .iter()
            .filter_map(|s| serde_json::from_value::<Algorithm>(Value::String(s.clone())).ok())
            .any(|alg| SAFE_ALGS.contains(&alg));
        if !acceptable {
            bail!(
                "provider advertises no ID token signing algorithm we accept (advertised: {})",
                meta.id_token_signing_alg_values_supported.join(", ")
            );
        }
    }
    let parsed_redirect = Url::parse(&args.redirect_uri).context("invalid redirect URI")?;
    if parsed_redirect.scheme() != "http" {
        bail!(
            "redirect URI must use http on a loopback address, got {:?}",
            args.redirect_uri
        );
    }
    let host = parsed_redirect.host_str().unwrap_or("");
    if !LOOPBACK_HOSTS.contains(&host) {
        bail!(
            "redirect URI host must be one of {}, got {host:?}",
            LOOPBACK_HOSTS.join(", ")
        );
    }
    let port = parsed_redirect
        .port()
        .context("redirect URI must include the port")?;
    let path = parsed_redirect.path().to_owned();

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let state = CsrfToken::new_random();
    let nonce = CsrfToken::new_random().secret().clone();

    let mut auth_url = Url::parse(&meta.authorization_endpoint)?;
    auth_url
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &args.client_id)
        .append_pair("redirect_uri", &args.redirect_uri)
        .append_pair("scope", "openid profile email offline_access groups")
        .append_pair("state", state.secret())
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge", pkce_challenge.as_str())
        .append_pair("code_challenge_method", "S256");
    let listener = bind_callback_listener(host, port).await?;
    eprintln!("listening on {}", args.redirect_uri);

    // Open browser; always print URL as fallback.
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(auth_url.as_str())
        .spawn()
        .ok();
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(auth_url.as_str())
        .spawn()
        .ok();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    eprintln!("automatic browser open is not supported on this platform!");

    eprintln!("\nOpen this URL if no browser window appeared:\n\n{auth_url}\n");

    let (code, got_state) = wait_for_callback(listener, &path, args.callback_timeout).await?;
    if !ct_str_eq(&got_state, state.secret()) {
        bail!("state mismatch; discarding authorization code");
    }

    let params = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", args.redirect_uri.as_str()),
        ("client_id", args.client_id.as_str()),
        ("code_verifier", pkce_verifier.secret()),
    ];

    // The key must exist before the exchange — a token cannot be bound to a key
    // that did not help mint it — so slimctl generates it here. This is the
    // *identity* key (`K_pop`), not an MLS key: the app's MLS leaf keys are
    // generated by `mls-rs` at runtime and attested under this one, which is why
    // they can rotate without another login.
    //
    // Always P-256, independent of the `curve25519` build feature: only JOSE
    // constrains this key, and the MLS ciphersuite no longer has a say.
    let identity_keys = match &args.dpop_credentials_file {
        Some(_) => Some(
            slim_auth::utils::generate_identity_signature_keys()
                .context("generating the identity key for DPoP binding")?,
        ),
        None => None,
    };

    let token_resp: Value = slim_auth::oidc::post_token_request_with_dpop(
        &http,
        &meta.token_endpoint,
        &params,
        identity_keys.as_ref(),
        false,
    )
    .await
    .context("token exchange")?;

    let id_token = token_resp["id_token"]
        .as_str()
        .context("missing id_token")?
        .to_owned();
    validate_id_token(&http, &meta, &id_token, &args.client_id, &nonce).await?;

    // Confirm the IdP honoured the proof before saving. With DPoP disabled it
    // returns a valid *unbound* token, which would instead fail at the first MLS
    // join, on a peer's machine.
    let (identity_private_key, identity_public_key) = match &identity_keys {
        Some((private_key, public_key)) => {
            let access_token = token_resp["access_token"]
                .as_str()
                .context("provider returned no access_token to bind")?;
            let expected = slim_auth::dpop::jwk_thumbprint(public_key)
                .context("computing the identity key thumbprint")?;
            match slim_auth::dpop::token_confirmation(access_token) {
                Some(jkt) if jkt == expected => {}
                Some(jkt) => bail!(
                    "provider bound the token to a different key (cnf.jkt {jkt:?}, expected {expected:?})"
                ),
                None => bail!(
                    "provider issued a token with no cnf.jkt: DPoP is not enabled for this \
                     client or realm (Keycloak: enable \"OAuth 2.0 DPoP\"). Omit \
                     --dpop-credentials-file for a plain bearer login."
                ),
            }
            (
                Some(BASE64.encode(private_key)),
                Some(BASE64.encode(public_key)),
            )
        }
        None => (None, None),
    };

    let creds = crate::config::OidcCredentials {
        id_token: id_token.to_string(),
        access_token: token_resp["access_token"].as_str().map(str::to_owned),
        refresh_token: token_resp["refresh_token"].as_str().map(str::to_owned),
        client_id: args.client_id.clone(),
        issuer: meta.issuer.clone(),
        token_endpoint: meta.token_endpoint.clone(),
        identity_private_key,
        identity_public_key,
    };

    // A named store is one app's identity, so it gets the credentials and
    // nothing else: writing the bearer token or the connection config would
    // retarget slimctl itself, and clobber whatever another app's login left.
    if let Some(path) = args.dpop_credentials_file.as_deref() {
        let path = std::path::PathBuf::from(path);
        crate::config::save_credentials_to(&path, &creds)?;
        eprintln!("Credentials saved to {}", path.display());
        eprintln!(
            "Point one app at it with `credentials_file: {}` in its OIDC identity config, \
             or SLIM_CREDENTIALS_FILE. slimctl's own auth is unchanged.",
            path.display()
        );
        return Ok(());
    }

    crate::config::save_credentials(&creds)?;
    let creds_path = crate::config::credentials_file_path()?;
    eprintln!("Credentials saved to {}", creds_path.display());

    // Point the connection config at the credential we just obtained, so
    // `slimctl node|controller|channel-manager` authenticate with no extra flags.
    let written = crate::config::save_login_auth(&creds, config_file, server)?;
    eprintln!(
        "Connection config {} points at the access token in {}.",
        written.config_path.display(),
        written.access_token_path.display()
    );
    if server.is_none() {
        eprintln!(
            "No --server given, so no endpoint was recorded: pass --server, or run \
             `slimctl config set server <host:port>`."
        );
    }
    eprintln!("That token expires; re-run `slimctl login` when commands stop authenticating.");
    match &written.refresh_token_path {
        Some(path) => eprintln!(
            "Refresh token saved to {} for long-lived processes (set `refresh_token_file` in their \
             OIDC auth config). slimctl does not use it.",
            path.display()
        ),
        None => eprintln!("No refresh token was issued by the provider."),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Wrap LoginArgs in a throwaway top-level parser so we can exercise clap
    /// validation without going through the full `Cli` struct.
    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        args: LoginArgs,
    }

    fn parse_ok(argv: &[&str]) -> LoginArgs {
        TestCli::try_parse_from(argv)
            .unwrap_or_else(|e| panic!("expected parse success for {argv:?}, got: {e}"))
            .args
    }

    fn parse_err(argv: &[&str]) -> clap::Error {
        match TestCli::try_parse_from(argv) {
            Err(e) => e,
            Ok(_) => panic!("expected parse failure for {argv:?}"),
        }
    }

    const REQUIRED: &[&str] = &[
        "test",
        "--client-id",
        "myclient",
        "--discovery-uri",
        "https://example.com/.well-known/openid-configuration",
    ];

    #[test]
    fn check_scheme_requires_https() {
        assert!(check_scheme("https://idp.example.com/realms/slim").is_ok());
        for bad in [
            "http://127.0.0.1:8080/realms/slim",
            "http://idp.example.com/realms/slim",
            "file:///etc/passwd",
            "ftp://idp.example.com/",
        ] {
            assert!(check_scheme(bad).is_err(), "should reject {bad}");
        }
    }

    /// One flag carries both decisions: DPoP binding and where the store goes.
    /// A bound login therefore cannot land in the default store, which the
    /// default store, which slimctl itself authenticates with.
    #[test]
    fn dpop_credentials_file_selects_an_app_identity() {
        let args = parse_ok(&[REQUIRED, &["--dpop-credentials-file", "/tmp/ci.yaml"]].concat());
        assert_eq!(args.dpop_credentials_file.as_deref(), Some("/tmp/ci.yaml"));
    }

    /// Absent means a plain bearer login to `~/.slimctl/credentials.yaml`.
    #[test]
    fn plain_login_by_default() {
        assert!(parse_ok(REQUIRED).dpop_credentials_file.is_none());
    }

    #[test]
    fn required_args_accepted() {
        let args = parse_ok(REQUIRED);
        assert_eq!(args.client_id, "myclient");
        assert_eq!(
            args.discovery_uri,
            "https://example.com/.well-known/openid-configuration"
        );
    }

    #[test]
    fn optional_args_accepted() {
        let args = parse_ok(&[
            "test",
            "--client-id",
            "myclient",
            "--discovery-uri",
            "https://example.com/.well-known/openid-configuration",
            "--redirect-uri",
            "http://127.0.0.1:9999/cb",
            "--callback-timeout",
            "60",
        ]);
        assert_eq!(args.redirect_uri, "http://127.0.0.1:9999/cb");
        assert_eq!(args.callback_timeout, 60);
    }

    #[test]
    fn defaults() {
        let args = parse_ok(REQUIRED);
        assert_eq!(args.redirect_uri, "http://127.0.0.1:8250/callback");
        assert_eq!(args.callback_timeout, 300);
    }

    #[test]
    fn missing_client_id_fails() {
        let err = parse_err(&[
            "test",
            "--discovery-uri",
            "https://example.com/.well-known/openid-configuration",
        ]);
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn missing_discovery_uri_fails() {
        let err = parse_err(&["test", "--client-id", "myclient"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn callback_timeout_zero_fails() {
        let err = parse_err(&[
            "test",
            "--client-id",
            "myclient",
            "--discovery-uri",
            "https://example.com/.well-known/openid-configuration",
            "--callback-timeout",
            "0",
        ]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }
}
