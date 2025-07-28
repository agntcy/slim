// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

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

#[derive(Clone)]
pub(crate) enum IdentityProvider {
    StaticJwt(StaticTokenProvider),
    SharedSecret(SharedSecret),
    SignerJwt(SignerJwt),
}

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

#[derive(Clone)]
pub(crate) enum IdentityVerifier {
    Jwt(Box<VerifierJwt>),
    SharedSecret(SharedSecret),
}

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
    async fn verify<Claims>(
        &self,
        token: impl Into<String> + Send,
    ) -> Result<Claims, slim_auth::errors::AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        match self {
            IdentityVerifier::Jwt(verifier) => verifier.verify(token).await,
            IdentityVerifier::SharedSecret(secret) => secret.verify(token).await,
        }
    }

    fn try_verify<Claims>(
        &self,
        token: impl Into<String>,
    ) -> Result<Claims, slim_auth::errors::AuthError>
    where
        Claims: serde::de::DeserializeOwned + Send,
    {
        match self {
            IdentityVerifier::Jwt(verifier) => verifier.try_verify(token),
            IdentityVerifier::SharedSecret(secret) => secret.try_verify(token),
        }
    }
}
