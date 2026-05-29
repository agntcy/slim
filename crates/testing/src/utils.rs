// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Submodules containing actual implementations
#[cfg(target_os = "linux")]
pub mod spire_env;
pub mod token;

// Re-export shared secret helpers
pub use token::TEST_VALID_SECRET;

// Re-export JWT / OIDC helpers
pub use token::{TestClaims, generate_test_token, setup_oidc_mock_server, setup_test_jwt_resolver};

// Re-export SPIRE environment (Linux only)
#[cfg(target_os = "linux")]
pub use spire_env::SpireTestEnvironment;
