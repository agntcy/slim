// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! The identity options shared by the sender and the receiver: a shared secret, or a
//! `did:key` (an Ed25519 private key and the public keys of the apps it trusts).

use std::sync::Arc;

use clap::Args;
use slim_bindings::{App, Direction, KeyBytes, Name, Service, SlimError, did_key_of_private_key};

/// How the app authenticates: pass `--shared-secret`, or `--private-key` with at least one
/// `--trusted-key`.
#[derive(Args, Debug)]
pub struct IdentityArgs {
    /// Shared secret for authentication (every app of the mesh uses the same one)
    #[arg(
        long,
        conflicts_with_all = ["private_key", "trusted_key"],
        required_unless_present = "private_key"
    )]
    shared_secret: Option<String>,

    /// did:key identity: the app's Ed25519 private key, in base64 (the 32-byte seed, or a
    /// 48-byte PKCS#8 document). The app's identity is the did:key of this key.
    /// `examples/did_key/generate-keys.sh` generates keys.
    #[arg(
        long,
        env = "SLIM_DID_KEY_PRIVATE_KEY",
        hide_env_values = true,
        required_unless_present = "shared_secret"
    )]
    private_key: Option<String>,

    /// did:key identity: the base64 public key (32 bytes) of an app this one accepts.
    /// Repeat it for every peer. The app's own key is trusted without being listed.
    #[arg(long, requires = "private_key")]
    trusted_key: Vec<String>,
}

impl IdentityArgs {
    /// Create the app `name` on `service` with the chosen identity.
    pub async fn create_app(
        &self,
        service: &Service,
        name: Arc<Name>,
    ) -> Result<Arc<App>, Box<dyn std::error::Error>> {
        if let Some(private_key) = &self.private_key {
            let private_key = KeyBytes::Base64 {
                value: private_key.clone(),
            };
            let trusted_keys: Vec<KeyBytes> = self
                .trusted_key
                .iter()
                .map(|value| KeyBytes::Base64 {
                    value: value.clone(),
                })
                .collect();
            println!(
                "Identity: {} (trusting {} other key(s))",
                did_key_of_private_key(private_key.clone())?,
                trusted_keys.len()
            );
            Ok(service
                .create_app_with_did_key_async(
                    name,
                    private_key,
                    trusted_keys,
                    Direction::Bidirectional,
                )
                .await?)
        } else {
            let secret = self
                .shared_secret
                .clone()
                .ok_or_else(|| SlimError::InvalidArgument {
                    message: "pass --shared-secret, or --private-key and --trusted-key".into(),
                })?;
            println!("Identity: shared secret");
            Ok(service.create_app_with_secret_async(name, secret).await?)
        }
    }
}
