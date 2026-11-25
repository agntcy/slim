// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! The identity options shared by the sender and the receiver: a shared secret, or a
//! `did:key` (the app's private key as a PEM file, and the JWKS of the keys it trusts).

use std::path::PathBuf;
use std::sync::Arc;

use clap::Args;
use slim_bindings::{App, Direction, Name, Service, SlimError, did_key_of_private_key};

/// How the app authenticates: pass `--shared-secret`, or `--private-key` and
/// `--trusted-keys`.
#[derive(Args, Debug)]
pub struct IdentityArgs {
    /// Shared secret for authentication (every app of the mesh uses the same one)
    #[arg(
        long,
        conflicts_with_all = ["private_key", "trusted_keys"],
        required_unless_present = "private_key"
    )]
    shared_secret: Option<String>,

    /// did:key identity: a file with the app's Ed25519 private key, a PKCS#8 PEM document
    /// (as `openssl genpkey -algorithm ed25519` writes). The app's identity is the
    /// did:key of this key. `examples/did_key/generate-keys.sh` generates keys.
    #[arg(
        long,
        requires = "trusted_keys",
        required_unless_present = "shared_secret"
    )]
    private_key: Option<PathBuf>,

    /// did:key identity: a file with the JWKS of the keys this app trusts, one OKP
    /// Ed25519 JWK per app it accepts. It has to list the app's own key too.
    #[arg(long, requires = "private_key")]
    trusted_keys: Option<PathBuf>,
}

fn read(path: &PathBuf) -> Result<String, Box<dyn std::error::Error>> {
    Ok(
        std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?,
    )
}

impl IdentityArgs {
    /// Create the app `name` on `service` with the chosen identity.
    pub async fn create_app(
        &self,
        service: &Service,
        name: Arc<Name>,
    ) -> Result<Arc<App>, Box<dyn std::error::Error>> {
        if let (Some(private_key), Some(trusted_keys)) = (&self.private_key, &self.trusted_keys) {
            let private_key = read(private_key)?;
            println!("Identity: {}", did_key_of_private_key(&private_key)?);
            Ok(service
                .create_app_with_did_key_async(
                    name,
                    private_key,
                    read(trusted_keys)?,
                    Direction::Bidirectional,
                )
                .await?)
        } else {
            let secret = self
                .shared_secret
                .clone()
                .ok_or_else(|| SlimError::InvalidArgument {
                    message: "pass --shared-secret, or --private-key and --trusted-keys".into(),
                })?;
            println!("Identity: shared secret");
            Ok(service.create_app_with_secret_async(name, secret).await?)
        }
    }
}
