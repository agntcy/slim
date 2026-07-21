// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! FFI persistence configuration and persistence-aware app creation.
//!
//! Kept in its own module so it **composes with** (rather than modifies) the
//! other app-creation paths: both the provider-based
//! [`crate::service::Service::create_app_with_direction_async`] and any
//! config-file-based creation funnel through the service-layer
//! `create_app_with_direction_and_persistence`. When persistence lands in the
//! app/node config, `create_app_from_slim_config` gains a `persistence:` section
//! and threads it through the same entry point — no change needed here.

use std::sync::Arc;

use slim_auth::auth_provider::{AuthProvider, AuthVerifier};
use slim_auth::traits::{TokenProvider, Verifier};
use slim_datapath::api::ProtoName as SlimName;
use slim_service::Service as SlimService;

use crate::app::{App, Direction};
use crate::errors::SlimError;
use crate::get_runtime;
use crate::identity_config::{IdentityProviderConfig, IdentityVerifierConfig};
use crate::name::Name;
use crate::service::Service;

/// Where and how a session's MLS/state is persisted at rest.
///
/// Persistence is opt-in and app-level: pass this to
/// [`Service::create_app_with_persistence`] to get a restorable app.
#[derive(uniffi::Record, Clone, Debug)]
pub struct PersistenceConfig {
    /// Directory holding the encrypted store (one file per identity).
    pub path: String,

    /// Optional passphrase. When `None`, the encryption key is derived from the
    /// endpoint identity — stable across restarts but only as secret as the
    /// identity id; supply a passphrase for strong confidentiality.
    pub passphrase: Option<String>,
}

impl From<PersistenceConfig> for slim_persistence::PersistenceConfig {
    fn from(c: PersistenceConfig) -> Self {
        slim_persistence::PersistenceConfig {
            path: c.path.into(),
            encryption_key: c
                .passphrase
                .map(slim_persistence::MlsEncryptionKey::Passphrase),
        }
    }
}

#[uniffi::export]
impl Service {
    /// Create an app with restorable MLS/session state persisted under
    /// `persistence.path` (async).
    ///
    /// Mirror of [`Service::create_app_with_direction_async`] with persistence
    /// enabled. Restore the app's sessions after a restart with
    /// [`App::restore_sessions`].
    pub async fn create_app_with_persistence_async(
        &self,
        name: Arc<Name>,
        identity_provider_config: IdentityProviderConfig,
        identity_verifier_config: IdentityVerifierConfig,
        direction: Direction,
        persistence: PersistenceConfig,
    ) -> Result<Arc<App>, SlimError> {
        let slim_name: SlimName = name.as_ref().into();
        create_app_with_persistence_internal(
            slim_name,
            identity_provider_config,
            identity_verifier_config,
            self.inner.clone(),
            direction,
            persistence.into(),
        )
        .await
        .map(Arc::new)
    }

    /// Blocking wrapper around [`Service::create_app_with_persistence_async`].
    pub fn create_app_with_persistence(
        &self,
        name: Arc<Name>,
        identity_provider_config: IdentityProviderConfig,
        identity_verifier_config: IdentityVerifierConfig,
        direction: Direction,
        persistence: PersistenceConfig,
    ) -> Result<Arc<App>, SlimError> {
        get_runtime().block_on(self.create_app_with_persistence_async(
            name,
            identity_provider_config,
            identity_verifier_config,
            direction,
            persistence,
        ))
    }
}

/// Provider-based app creation with persistence. Deliberately separate from
/// `create_app_async_internal` so this stays additive; the only difference is
/// calling `create_app_with_direction_and_persistence`.
async fn create_app_with_persistence_internal(
    base_name: SlimName,
    identity_provider_config: IdentityProviderConfig,
    identity_verifier_config: IdentityVerifierConfig,
    service: Arc<SlimService>,
    direction: Direction,
    persistence: slim_persistence::PersistenceConfig,
) -> Result<App, SlimError> {
    let mut identity_provider: AuthProvider = identity_provider_config.try_into()?;
    let mut identity_verifier: AuthVerifier = identity_verifier_config.try_into()?;

    identity_provider.initialize().await?;
    identity_verifier.initialize().await?;
    let _identity_token = identity_provider.get_token()?;

    let (app, rx) = service.create_app_with_direction_and_persistence(
        &base_name,
        identity_provider,
        identity_verifier,
        direction.into(),
        Some(persistence),
    )?;

    Ok(App::from_parts(
        Arc::new(app),
        Arc::new(tokio::sync::RwLock::new(rx)),
        service,
    ))
}
