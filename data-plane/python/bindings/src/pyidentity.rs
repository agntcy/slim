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
// - PyAlgorithm: Supported JWT / signature algorithms.
// - PyKeyData: Source of key material (file path vs inline content).
// - PyKeyFormat: Format of the key material (PEM / JWK / JWKS).
// - PyKey: Composite describing an algorithm, format and key payload.
// - PyIdentityProvider: Strategies for producing tokens (static file,
//   signing with private key, or shared secret).
// - PyIdentityVerifier: Strategies for validating tokens (JWT or
//   shared secret).
//
// Typical Flow (Python):
//   1. Create a PyKey (if using a JWT signing or verification scenario)
//   2. Build a PyIdentityProvider (e.g. Jwt {...})
//   3. Build a PyIdentityVerifier (e.g. Jwt {...})
//   4. Pass provider + verifier into Slim.new(...)
//
// Error Handling:
//   Construction helpers will panic only in unrecoverable internal
//   builder misconfigurations (should not happen for valid user input).
//   Runtime token generation / verification errors surface as Python
//   exceptions when methods are invoked across the boundary.

use pyo3::prelude::*;
use pyo3_stub_gen::derive::{gen_stub_pyclass, gen_stub_pyclass_enum, gen_stub_pymethods};

use slim_auth::builder::JwtBuilder;
use slim_auth::jwt::Key;
use slim_auth::jwt::KeyFormat;
use slim_auth::jwt::SignerJwt;
use slim_auth::jwt::StaticTokenProvider;
use slim_auth::jwt::VerifierJwt;
use slim_auth::jwt::{Algorithm, KeyData};
use slim_auth::shared_secret::SharedSecret;
use slim_auth::traits::TokenProvider;
use slim_auth::traits::Verifier;

/// JWT / signature algorithms exposed to Python.
///
/// Maps 1:1 to `slim_auth::jwt::Algorithm`.
/// Provides stable integer values for stub generation / introspection.
#[gen_stub_pyclass_enum]
#[pyclass(eq, eq_int)]
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
/// accompanying `PyKeyFormat`.
#[gen_stub_pyclass_enum]
#[derive(Clone, PartialEq)]
#[pyclass(eq)]
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
            PyKeyData::Content { content } => KeyData::Str(content),
        }
    }
}

/// Supported key encoding formats.
///
/// Used during parsing / loading of provided key material.
#[gen_stub_pyclass_enum]
#[derive(Clone, PartialEq)]
#[pyclass(eq)]
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
/// * algorithm: `PyAlgorithm` to apply
/// * format: `PyKeyFormat` describing encoding
/// * key: `PyKeyData` where the actual bytes originate
#[gen_stub_pyclass]
#[pyclass]
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
    /// Construct a new `PyKey`.
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

/// Internal enum for token provisioning strategies.
#[derive(Clone)]
enum IdentityProvider {
    StaticJwt(StaticTokenProvider),
    SharedSecret(SharedSecret),
    SignerJwt(SignerJwt),
}

/// Python-facing identity provider definitions.
///
/// Variants:
/// * StaticJwt { path }: Load a token from a file (cached, static).
/// * Jwt { private_key, duration, issuer?, audience?, subject? }:
///     Dynamically sign tokens using provided private key with optional
///     standard JWT claims (iss, aud, sub) and a token validity duration.
/// * SharedSecret { identity, shared_secret }:
///     Symmetric token provider using a shared secret. Used mainly for testing.
///
/// Usage (Python):
/// ```python
/// key = PyKey(PyAlgorithm.RS256, PyKeyFormat.Pem, PyKeyData.File("private_key.pem"))
/// provider = PyIdentityProvider.Jwt(
///     private_key=key,
///     duration=datetime.timedelta(hours=1),
///     issuer="my-issuer",
///     audience=["svc-b"],
///     subject="svc-a"
/// )
/// ```
#[gen_stub_pyclass_enum]
#[derive(Clone, PartialEq)]
#[pyclass(eq)]
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
}

impl From<PyIdentityProvider> for IdentityProvider {
    fn from(value: PyIdentityProvider) -> Self {
        match value {
            PyIdentityProvider::StaticJwt { path } => IdentityProvider::StaticJwt(
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

                if let Some(issuer) = issuer {
                    builder = builder.issuer(issuer);
                }
                if let Some(audience) = audience {
                    builder = builder.audience(&audience);
                }
                if let Some(subject) = subject {
                    builder = builder.subject(subject);
                }

                IdentityProvider::SignerJwt(
                    builder
                        .private_key(&private_key.into())
                        .token_duration(duration)
                        .build()
                        .expect("Failed to build SignerJwt"),
                )
            }
            PyIdentityProvider::SharedSecret {
                identity,
                shared_secret,
            } => IdentityProvider::SharedSecret(SharedSecret::new(&identity, &shared_secret)),
        }
    }
}

impl TokenProvider for IdentityProvider {
    fn get_token(&self) -> Result<String, slim_auth::errors::AuthError> {
        match self {
            IdentityProvider::StaticJwt(provider) => provider.get_token(),
            IdentityProvider::SharedSecret(secret) => secret.get_token(),
            IdentityProvider::SignerJwt(signer) => signer.get_token(),
        }
    }
}

/// Internal enum for verification strategies.
#[derive(Clone)]
enum IdentityVerifier {
    Jwt(Box<VerifierJwt>),
    SharedSecret(SharedSecret),
}

/// Python-facing identity verifier definitions.
///
/// Variants:
/// * Jwt { public_key?, autoresolve, issuer?, audience?, subject?, require_* }:
///     Verifies tokens using a public key or via JWKS auto-resolution.
///     `require_iss`, `require_aud`, `require_sub` toggle mandatory presence
///     of the respective claims. `autoresolve=True` enables JWKS retrieval
///     (public_key must be omitted in that case).
/// * SharedSecret { identity, shared_secret }:
///     Verifies HMAC-style tokens generated with the same shared secret.
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
/// pub_key = PyKey(
///     PyAlgorithm.RS256,
///     PyKeyFormat.Pem,
///     PyKeyData.File("public_key.pem"),
/// )
/// verifier = PyIdentityVerifier.Jwt(
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
/// verifier = PyIdentityVerifier.Jwt(
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
/// verifier = PyIdentityVerifier.SharedSecret(
///     identity="service-a",
///     shared_secret="super-secret-value",
/// )
/// ```
///
/// Pairing with a provider when constructing Slim:
/// ```python
/// provider = PyIdentityProvider.SharedSecret(
///     identity="service-a",
///     shared_secret="super-secret-value",
/// )
/// slim = await Slim.new(local_name, provider, verifier)
/// ```
///
/// Enforcing strict claims (reject tokens missing aud/sub):
/// ```python
/// strict_verifier = PyIdentityVerifier.Jwt(
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
#[gen_stub_pyclass_enum]
#[derive(Clone, PartialEq)]
#[pyclass(eq)]
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

                IdentityVerifier::Jwt(Box::new(ret))
            }
            PyIdentityVerifier::SharedSecret {
                identity,
                shared_secret,
            } => IdentityVerifier::SharedSecret(SharedSecret::new(&identity, &shared_secret)),
        }
    }
}

#[async_trait::async_trait]
impl Verifier for IdentityVerifier {
    async fn verify(
        &self,
        token: impl Into<String> + Send,
    ) -> Result<(), slim_auth::errors::AuthError> {
        match self {
            IdentityVerifier::Jwt(verifier) => verifier.verify(token).await,
            IdentityVerifier::SharedSecret(secret) => secret.verify(token).await,
        }
    }

    fn try_verify(&self, token: impl Into<String>) -> Result<(), slim_auth::errors::AuthError> {
        match self {
            IdentityVerifier::Jwt(verifier) => verifier.try_verify(token),
            IdentityVerifier::SharedSecret(secret) => secret.try_verify(token),
        }
    }

    async fn get_claims<Claims>(
        &self,
        token: impl Into<String> + Send,
    ) -> Result<Claims, slim_auth::errors::AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        match self {
            IdentityVerifier::Jwt(verifier) => verifier.get_claims(token).await,
            IdentityVerifier::SharedSecret(secret) => secret.get_claims(token).await,
        }
    }

    fn try_get_claims<Claims>(
        &self,
        token: impl Into<String>,
    ) -> Result<Claims, slim_auth::errors::AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        match self {
            IdentityVerifier::Jwt(verifier) => verifier.try_get_claims(token),
            IdentityVerifier::SharedSecret(secret) => secret.try_get_claims(token),
        }
    }
}
