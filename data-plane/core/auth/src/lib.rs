// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

pub mod builder;
pub mod errors;
pub mod file_watcher;
pub mod jwt;
pub mod jwt_middleware;
pub mod oauth2;
pub mod resolver;
pub mod shared_secret;
pub mod testutils;
pub mod traits;

// Re-export main authentication traits
pub use traits::{TokenProvider, Verifier, Signer, StandardClaims};

// Re-export OAuth2 components for easier access
pub use oauth2::{
    OAuth2ClientCredentialsConfig,
    OAuth2TokenProvider,
    OAuth2Verifier,
};

// Re-export JWT components for convenience
pub use jwt::{SignerJwt, VerifierJwt, StaticTokenProvider};

// Re-export builder for easy access
pub use builder::JwtBuilder;

// Re-export common error type
pub use errors::AuthError;
