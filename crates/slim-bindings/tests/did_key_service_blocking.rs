// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! `Service::create_app_with_did_key` works without the caller setting up a Tokio runtime or the TLS crypto
//! provider, and runs without a Tokio runtime of the caller. Its own test binary: the
//! provider is process-global.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use slim_bindings::{Direction, Name, Service};
use slim_testing::keys::{ed25519_jwks_of, ed25519_pem_of_seed};

// RFC 8032 section 7.1, test 1, and its public key as a JWK `x` (RFC 8037 appendix A).
const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const PUBLIC: &str = "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo";

fn app_name() -> Arc<Name> {
    Arc::new(Name::new("org".into(), "ns".into(), "did-app".into()))
}

#[test]
fn service_create_app_with_did_key_works_without_setup() {
    let app = Service::new("did-key-service-blocking".into())
        .create_app_with_did_key(
            app_name(),
            ed25519_pem_of_seed(SEED),
            ed25519_jwks_of(&[PUBLIC]),
            Direction::Bidirectional,
        )
        .expect("an app with a did:key identity");
    assert!(!app.id().is_empty());
}
