// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! The async did:key APIs run on the runtime of the bindings, so they work when they are
//! polled outside Tokio, as UniFFI does for a foreign-language caller. (The Rust
//! `slim_service` API does not: it panics without a Tokio runtime.)

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use slim_bindings::{App, Direction, Name, Service};
use slim_testing::keys::{ed25519_jwks_of, ed25519_pem_of_seed};

// RFC 8032 section 7.1, test 1, and its public key as a JWK `x` (RFC 8037 appendix A).
const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const PUBLIC: &str = "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo";

fn app_name() -> Arc<Name> {
    Arc::new(Name::new("org".into(), "ns".into(), "did-app".into()))
}

#[test]
fn service_create_app_with_did_key_async_works_without_tokio() {
    let service = Service::new("did-key-service-no-tokio".into());
    let app = futures::executor::block_on(service.create_app_with_did_key_async(
        app_name(),
        ed25519_pem_of_seed(SEED),
        ed25519_jwks_of(&[PUBLIC]),
        Direction::Bidirectional,
    ))
    .expect("an app with a did:key identity");
    assert!(!app.id().is_empty());
}

#[test]
fn app_new_with_did_key_async_works_without_tokio() {
    let app = futures::executor::block_on(App::new_with_did_key_async(
        app_name(),
        ed25519_pem_of_seed(SEED),
        ed25519_jwks_of(&[PUBLIC]),
        Direction::Bidirectional,
    ))
    .expect("an app with a did:key identity");
    assert!(!app.id().is_empty());
}
