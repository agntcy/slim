// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! `create_app_with_did_key` must not need the caller to install the TLS crypto
//! provider. Each did:key API has its own test binary (see the files next to this one), so
//! that no other test installs the provider first: it is process-global.

use slim_config::component::id::ID;
use slim_datapath::api::ProtoName;
use slim_service::service::{Service, ServiceBuilder};
use slim_testing::keys::{ed25519_jwks_of, ed25519_pem_of_seed};

// RFC 8032 section 7.1, test 1, and its public key as a JWK `x` (RFC 8037 appendix A).
const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const PUBLIC: &str = "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo";

fn service(name: &str) -> Service {
    Service::new(ID::new_with_name(ServiceBuilder::kind(), name).unwrap())
}

fn app_name() -> ProtoName {
    ProtoName::from_strings(["org", "ns", "did-app"])
}

#[tokio::test]
async fn create_app_with_did_key_installs_the_crypto_provider() {
    let result = service("svc-did-key-no-provider")
        .create_app_with_did_key(
            &app_name(),
            &ed25519_pem_of_seed(SEED),
            &ed25519_jwks_of(&[PUBLIC]),
        )
        .await;
    assert!(result.is_ok(), "{:?}", result.err());
}
