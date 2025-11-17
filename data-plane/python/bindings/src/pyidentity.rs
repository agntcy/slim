// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0
//
// Identity & cryptography related Python bindings.
// These pyclasses/enums provide a Python-facing configuration surface
// for supplying identity (token generation) and verification logic
// to the Slim service. They mirror internal Rust types and are
// converted transparently across the FFI boundary.
//
// Overview:
// - Algorithm: Supported JWT / signature algorithms.
// - KeyData: Source of key material (file path vs inline content).
// - KeyFormat: Format of the key material (PEM / JWK / JWKS).
// - Key: Composite describing an algorithm, format and key payload.
// - IdentityProvider: Strategies for producing tokens (static file,
//   signing with private key, or shared secret).
// - IdentityVerifier: Strategies for validating tokens (JWT or
//   shared secret).
//
// Typical Flow (Python):
//   1. Create a Key (if using a JWT signing or verification scenario)
//   2. Build an IdentityProvider (e.g. Jwt {...})
//   3. Build an IdentityVerifier (e.g. Jwt {...})
//   4. Pass provider + verifier into Slim.new(...)
//
// Error Handling:
//   Construction helpers will panic only in unrecoverable internal
//   builder misconfigurations (should not happen for valid user input).
//   Runtime token generation / verification errors surface as Python
//   exceptions when methods are invoked across the boundary.

use pyo3::prelude::*;
use pyo3_stub_gen::derive::{
    gen_stub_pyclass, gen_stub_pyclass_complex_enum, gen_stub_pyclass_enum, gen_stub_pymethods,
};

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::builder::JwtBuilder;
use slim_auth::jwt::Key;
use slim_auth::jwt::KeyFormat;

use slim_auth::jwt::StaticTokenProvider;

use slim_auth::jwt::{Algorithm, KeyData};
use slim_auth::shared_secret::SharedSecret;

/// JWT / signature algorithms exposed to Python.
///
/// Maps 1:1 to `slim_auth::jwt::Algorithm`.
/// Provides stable integer values for stub generation / introspection.
#[gen_stub_pyclass_enum]
#[pyclass(name = "Algorithm", eq, eq_int)]
#[derive(PartialEq, Clone)]
pub(crate) enum PyAlgorithm {
    #[pyo3(name = "HS256")]
    HS256 = Algorithm::HS256 as isize,
    #[pyo3(name = "HS384")]
    HS384 = Algorithm::HS384 as isize,
    #[pyo3(name = "HS512")]
    HS512 = Algorithm::HS512 as isize,
    #[pyo3(name = "RS256")]
    RS256 = Algorithm::RS256 as isize,
    #[pyo3(name = "RS384")]
    RS384 = Algorithm::RS384 as isize,
    #[pyo3(name = "RS512")]
    RS512 = Algorithm::RS512 as isize,
    #[pyo3(name = "PS256")]
    PS256 = Algorithm::PS256 as isize,
    #[pyo3(name = "PS384")]
    PS384 = Algorithm::PS384 as isize,
    #[pyo3(name = "PS512")]
    PS512 = Algorithm::PS512 as isize,
    #[pyo3(name = "ES256")]
    ES256 = Algorithm::ES256 as isize,
    #[pyo3(name = "ES384")]
    ES384 = Algorithm::ES384 as isize,
    #[pyo3(name = "EdDSA")]
    EdDSA = Algorithm::EdDSA as isize,
}

impl From<PyAlgorithm> for Algorithm {
    fn from(value: PyAlgorithm) -> Self {
        match value {
            PyAlgorithm::HS256 => Algorithm::HS256,
            PyAlgorithm::HS384 => Algorithm::HS384,
            PyAlgorithm::HS512 => Algorithm::HS512,
            PyAlgorithm::RS256 => Algorithm::RS256,
            PyAlgorithm::RS384 => Algorithm::RS384,
            PyAlgorithm::RS512 => Algorithm::RS512,
            PyAlgorithm::PS256 => Algorithm::PS256,
            PyAlgorithm::PS384 => Algorithm::PS384,
            PyAlgorithm::PS512 => Algorithm::PS512,
            PyAlgorithm::ES256 => Algorithm::ES256,
            PyAlgorithm::ES384 => Algorithm::ES384,
            PyAlgorithm::EdDSA => Algorithm::EdDSA,
        }
    }
}

/// Key material origin.
///
/// Either a path on disk (`File`) or inline string content (`Content`)
/// containing the encoded key. The interpretation depends on the
/// accompanying `KeyFormat`.
#[gen_stub_pyclass_complex_enum]
#[derive(Clone, PartialEq)]
#[pyclass(name = "KeyData", eq)]
pub(crate) enum PyKeyData {
    #[pyo3(constructor = (path))]
    File { path: String },
    #[pyo3(constructor = (content))]
    Content { content: String },
}

impl From<PyKeyData> for KeyData {
    fn from(value: PyKeyData) -> Self {
        match value {
            PyKeyData::File { path } => KeyData::File(path),
            PyKeyData::Content { content } => KeyData::Data(content),
        }
    }
}

/// Supported key encoding formats.
///
/// Used during parsing / loading of provided key material.
#[gen_stub_pyclass_enum]
#[derive(Clone, PartialEq)]
#[pyclass(name = "KeyFormat", eq)]
pub(crate) enum PyKeyFormat {
    Pem,
    Jwk,
    Jwks,
}

impl From<PyKeyFormat> for KeyFormat {
    fn from(value: PyKeyFormat) -> Self {
        match value {
            PyKeyFormat::Pem => KeyFormat::Pem,
            PyKeyFormat::Jwk => KeyFormat::Jwk,
            PyKeyFormat::Jwks => KeyFormat::Jwks,
        }
    }
}

/// Composite key description used for signing or verification.
///
/// Fields:
/// * algorithm: `Algorithm` to apply
/// * format: `KeyFormat` describing encoding
/// * key: `KeyData` where the actual bytes originate
#[gen_stub_pyclass]
#[pyclass(name = "Key")]
#[derive(Clone, PartialEq)]
pub(crate) struct PyKey {
    #[pyo3(get, set)]
    algorithm: PyAlgorithm,

    #[pyo3(get, set)]
    format: PyKeyFormat,

    #[pyo3(get, set)]
    key: PyKeyData,
}

#[gen_stub_pymethods]
#[pymethods]
impl PyKey {
    /// Construct a new `Key`.
    ///
    /// Args:
    ///   algorithm: Algorithm used for signing / verification.
    ///   format: Representation format (PEM/JWK/JWKS).
    ///   key: Source (file vs inline content).
    #[new]
    pub fn new(algorithm: PyAlgorithm, format: PyKeyFormat, key: PyKeyData) -> Self {
        PyKey {
            algorithm,
            format,
            key,
        }
    }
}

impl From<PyKey> for Key {
    fn from(value: PyKey) -> Self {
        Key {
            algorithm: value.algorithm.into(),
            format: value.format.into(),
            key: value.key.into(),
        }
    }
}

/// Internal type alias for token provisioning strategies.
///
/// This reuses the core `AuthProvider` enum from `slim_auth::auth_provider`
/// to avoid duplication and ensure consistency across the system.
pub(crate) type IdentityProvider = AuthProvider;

/// Python-facing identity provider definitions.
///
/// Variants:
/// * StaticJwt { path }: Load a token from a file (cached, static).
/// * Jwt { private_key, duration, issuer?, audience?, subject? }:
///     Dynamically sign tokens using provided private key with optional
///     standard JWT claims (iss, aud, sub) and a token validity duration.
/// * SharedSecret { identity, shared_secret }:
///     Symmetric token provider using a shared secret. Used mainly for testing.
/// * Spire { socket_path=None, target_spiffe_id=None, jwt_audiences=None }:
///     SPIRE-based provider retrieving SPIFFE JWT SVIDs (non-Windows only; requires SPIRE agent socket).
///
/// Examples (Python):
///
/// Static (pre-issued) JWT token loaded from a file:
/// ```python
/// from slim_bindings import IdentityProvider
///
/// provider = IdentityProvider.StaticJwt(path="service.token")
/// # 'provider.get_token()' (internally) will manage reloading of the file if it changes.
/// ```
///
/// Dynamically signed JWT using a private key (claims + duration):
/// ```python
/// from slim_bindings import (
///     IdentityProvider, Key, Algorithm, KeyFormat, KeyData
/// )
/// import datetime
///
/// signing_key = Key(
///     algorithm=Algorithm.RS256,
///     format=KeyFormat.Pem,
///     key=KeyData.File("private_key.pem"),
/// )
///
/// provider = IdentityProvider.Jwt(
///     private_key=signing_key,
///     duration=datetime.timedelta(minutes=30),
///     issuer="my-issuer",
///     audience=["downstream-svc"],
///     subject="svc-a",
/// )
/// ```
///
/// Shared secret token provider for tests / local development:
/// ```python
/// from slim_bindings import IdentityProvider
///
/// provider = IdentityProvider.SharedSecret(
///     identity="svc-a",
///     shared_secret="not-for-production",
/// )
/// ```
///
/// End-to-end example pairing with a verifier:
/// ```python
/// # For a simple shared-secret flow:
/// from slim_bindings import IdentityProvider, IdentityVerifier
///
/// provider = IdentityProvider.SharedSecret(identity="svc-a", shared_secret="dev-secret")
/// verifier = IdentityVerifier.SharedSecret(identity="svc-a", shared_secret="dev-secret")
///
/// # Pass both into Slim.new(local_name, provider, verifier)
/// ```
///
/// Jwt variant quick start (full):
/// ```python
/// import datetime
/// from slim_bindings import (
///     IdentityProvider, IdentityVerifier,
///     Key, Algorithm, KeyFormat, KeyData
/// )
///
/// key = Key(Algorithm.RS256, KeyFormat.Pem, KeyData.File("private_key.pem"))
/// provider = IdentityProvider.Jwt(
///     private_key=key,
///     duration=datetime.timedelta(hours=1),
///     issuer="my-issuer",
///     audience=["svc-b"],
///     subject="svc-a"
/// )
/// # Verifier would normally use the corresponding public key (IdentityVerifier.Jwt).
/// ```
#[gen_stub_pyclass_complex_enum]
#[derive(Clone)]
#[pyclass(name = "IdentityProvider")]
pub(crate) enum PyIdentityProvider {
    #[pyo3(constructor = (path))]
    StaticJwt { path: String },
    #[pyo3(constructor = (private_key, duration, issuer=None, audience=None, subject=None))]
    Jwt {
        private_key: PyKey,
        duration: std::time::Duration,
        issuer: Option<String>,
        audience: Option<Vec<String>>,
        subject: Option<String>,
    },
    #[pyo3(constructor = (identity, shared_secret))]
    SharedSecret {
        identity: String,
        shared_secret: String,
    },
    #[pyo3(constructor = (socket_path=None, target_spiffe_id=None, jwt_audiences=None))]
    Spire {
        socket_path: Option<String>,
        target_spiffe_id: Option<String>,
        jwt_audiences: Option<Vec<String>>,
    },
}

impl From<PyIdentityProvider> for IdentityProvider {
    fn from(value: PyIdentityProvider) -> Self {
        match value {
            PyIdentityProvider::StaticJwt { path } => AuthProvider::StaticToken(
                StaticTokenProvider::from(JwtBuilder::new().token_file(path).build().unwrap()),
            ),
            PyIdentityProvider::Jwt {
                private_key,
                duration,
                issuer,
                audience,
                subject,
            } => {
                let mut builder = JwtBuilder::new();

                if let Some(iss) = issuer {
                    builder = builder.issuer(iss);
                }
                if let Some(aud) = audience {
                    builder = builder.audience(&aud);
                }
                if let Some(sub) = subject {
                    builder = builder.subject(sub);
                }

                let signer = builder
                    .private_key(&private_key.into())
                    .token_duration(duration)
                    .build()
                    .expect("Failed to build SignerJwt");

                AuthProvider::JwtSigner(signer)
            }
            PyIdentityProvider::SharedSecret {
                identity,
                shared_secret,
            } => AuthProvider::SharedSecret(SharedSecret::new(&identity, &shared_secret)),
            PyIdentityProvider::Spire {
                socket_path,
                target_spiffe_id,
                jwt_audiences,
            } => {
                #[cfg(not(target_family = "windows"))]
                {
                    let mut builder = slim_auth::spire::SpireIdentityManager::builder();
                    if let Some(sp) = socket_path {
                        builder = builder.with_socket_path(sp);
                    }
                    if let Some(id) = target_spiffe_id {
                        builder = builder.with_target_spiffe_id(id);
                    }
                    if let Some(auds) = jwt_audiences {
                        builder = builder.with_jwt_audiences(auds);
                    }
                    let mgr = builder.build();
                    // Not initialized yet; caller may invoke .init() before use.
                    AuthProvider::Spire(mgr)
                }
                #[cfg(target_family = "windows")]
                {
                    let _ = (socket_path, target_spiffe_id, jwt_audiences);
                    panic!(
                        "SPIRE identity provider is not supported on Windows. SPIRE requires Unix domain sockets which are not available on Windows platforms."
                    );
                }
            }
        }
    }
}

/// Internal type alias for token verification strategies.
///
/// This reuses the core `AuthVerifier` enum from `slim_auth::auth_provider`
/// to avoid duplication and ensure consistency across the system.
pub(crate) type IdentityVerifier = AuthVerifier;

/// Python-facing identity verifier definitions.
///
/// Variants:
/// * Jwt { public_key?, autoresolve, issuer?, audience?, subject?, require_* }:
///     Verifies tokens using a public key or via JWKS auto-resolution.
///     `require_iss`, `require_aud`, `require_sub` toggle mandatory presence
///     of the respective claims. `autoresolve=True` enables JWKS retrieval
///     (public_key must be omitted in that case).
/// * SharedSecret { identity, shared_secret }:
///     Verifies tokens generated with the same shared secret.
/// * Spire { socket_path=None, target_spiffe_id=None, jwt_audiences=None }:
///     SPIRE-based JWT SVID verifier (non-Windows only). Uses SPIRE Workload API
///     bundles to validate SPIFFE JWT SVIDs. Requires an initialized SPIRE
///     identity manager. (Underlying AuthVerifier support must exist.)
///
/// JWKS Auto-Resolve:
///   When `autoresolve=True`, the verifier will attempt to resolve keys
///   dynamically (e.g. from a JWKS endpoint) if supported by the underlying
///   implementation.
///
/// Safety:
///   A direct panic occurs if neither `public_key` nor `autoresolve=True`
///   is provided for the Jwt variant (invalid configuration).
///
/// Autoresolve key selection (concise algorithm):
/// 1. If a static JWKS was injected, use it directly.
/// 2. Else if a cached JWKS for the issuer exists and is within TTL, use it.
/// 3. Else discover JWKS:
///    - Try {issuer}/.well-known/openid-configuration for "jwks_uri"
///    - Fallback to {issuer}/.well-known/jwks.json
/// 4. Fetch & cache the JWKS (default TTL ~1h unless overridden).
/// 5. If JWT header has 'kid', pick the matching key ID; otherwise choose the
///    first key whose algorithm matches the token header's alg.
/// 6. Convert JWK -> DecodingKey and verify signature; then enforce required
///    claims (iss/aud/sub) per the require_* flags.
///
/// # Examples (Python)
///
/// Basic JWT verification with explicit public key:
/// ```python
/// pub_key = Key(
///     Algorithm.RS256,
///     KeyFormat.Pem,
///     KeyData.File("public_key.pem"),
/// )
/// verifier = IdentityVerifier.Jwt(
///     public_key=pub_key,
///     autoresolve=False,
///     issuer="my-issuer",
///     audience=["service-b"],
///     subject="service-a",
///     require_iss=True,
///     require_aud=True,
///     require_sub=True,
/// )
/// ```
///
/// Auto-resolving JWKS (no public key provided):
/// ```python
/// # The underlying implementation must know how / where to resolve JWKS.
/// verifier = IdentityVerifier.Jwt(
///     public_key=None,
///     autoresolve=True,
///     issuer="https://auth.example.com",
///     audience=["svc-cluster"],
///     subject=None,
///     require_iss=True,
///     require_aud=True,
///     require_sub=False,
/// )
/// ```
///
/// Shared secret verifier (symmetric):
/// ```python
/// verifier = IdentityVerifier.SharedSecret(
///     identity="service-a",
///     shared_secret="super-secret-value",
/// )
/// ```
///
/// Pairing with a provider when constructing Slim:
/// ```python
/// provider = IdentityProvider.SharedSecret(
///     identity="service-a",
///     shared_secret="super-secret-value",
/// )
/// slim = await Slim.new(local_name, provider, verifier)
/// ```
///
/// Enforcing strict claims (reject tokens missing aud/sub):
/// ```python
/// strict_verifier = IdentityVerifier.Jwt(
///     public_key=pub_key,
///     autoresolve=False,
///     issuer="my-issuer",
///     audience=["service-a"],
///     subject="service-a",
///     require_iss=True,
///     require_aud=True,
///     require_sub=True,
/// )
/// ```
#[gen_stub_pyclass_complex_enum]
#[derive(Clone, PartialEq)]
#[pyclass(name = "IdentityVerifier", eq)]
pub(crate) enum PyIdentityVerifier {
    #[pyo3(constructor = (public_key=None, autoresolve=false, issuer=None, audience=None, subject=None, require_iss=false, require_aud=false, require_sub=false))]
    Jwt {
        public_key: Option<PyKey>,
        autoresolve: bool,
        issuer: Option<String>,
        audience: Option<Vec<String>>,
        subject: Option<String>,
        require_iss: bool,
        require_aud: bool,
        require_sub: bool,
    },
    #[pyo3(constructor = (identity, shared_secret))]
    SharedSecret {
        identity: String,
        shared_secret: String,
    },
    #[pyo3(constructor = (socket_path=None, target_spiffe_id=None, jwt_audiences=None))]
    Spire {
        socket_path: Option<String>,
        target_spiffe_id: Option<String>,
        jwt_audiences: Option<Vec<String>>,
    },
}

impl From<PyIdentityVerifier> for IdentityVerifier {
    fn from(value: PyIdentityVerifier) -> Self {
        match value {
            PyIdentityVerifier::Jwt {
                public_key,
                autoresolve,
                issuer,
                audience,
                subject,
                require_iss,
                require_aud,
                require_sub,
            } => {
                let mut builder = JwtBuilder::new();

                if let Some(issuer) = issuer {
                    builder = builder.issuer(issuer);
                }

                if let Some(audience) = audience {
                    builder = builder.audience(&audience);
                }

                if let Some(subject) = subject {
                    builder = builder.subject(subject);
                }

                if require_iss {
                    builder = builder.require_iss();
                }

                if require_aud {
                    builder = builder.require_aud();
                }

                if require_sub {
                    builder = builder.require_sub();
                }

                builder = builder.require_exp();

                let ret = match (public_key, autoresolve) {
                    (Some(key), _) => builder.public_key(&key.into()).build().unwrap(),
                    (_, true) => builder.auto_resolve_keys(true).build().unwrap(),
                    (_, _) => panic!("Public key must be provided for JWT verifier"),
                };

                AuthVerifier::JwtVerifier(ret)
            }
            PyIdentityVerifier::SharedSecret {
                identity,
                shared_secret,
            } => AuthVerifier::SharedSecret(SharedSecret::new(&identity, &shared_secret)),
            PyIdentityVerifier::Spire {
                socket_path,
                target_spiffe_id,
                jwt_audiences,
            } => {
                #[cfg(not(target_family = "windows"))]
                {
                    let mut builder = slim_auth::spire::SpireIdentityManager::builder();
                    if let Some(sp) = socket_path {
                        builder = builder.with_socket_path(sp);
                    }
                    if let Some(id) = target_spiffe_id {
                        builder = builder.with_target_spiffe_id(id);
                    }
                    if let Some(auds) = jwt_audiences {
                        builder = builder.with_jwt_audiences(auds);
                    }
                    let mgr = builder.build();
                    // Not initialized yet; caller may invoke .init() before use.
                    AuthVerifier::Spire(mgr)
                }
                #[cfg(target_family = "windows")]
                {
                    let _ = (socket_path, target_spiffe_id, jwt_audiences);
                    panic!(
                        "SPIRE identity verifier is not supported on Windows. SPIRE requires Unix domain sockets which are not available on Windows platforms."
                    );
                }
            }
        }
    }
}
