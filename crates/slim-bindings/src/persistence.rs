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
use slim_datapath::api::ProtoName as SlimName;
use slim_service::Service as SlimService;

use crate::app::{App, Direction};
use crate::errors::SlimError;
use crate::get_runtime;
use crate::identity_config::{IdentityProviderConfig, IdentityVerifierConfig};
use crate::name::Name;
use crate::service::{Service, create_app_from_identity};

/// Where and how a session's MLS/state is persisted at rest.
///
/// Persistence is opt-in and app-level: pass this to
/// [`Service::create_app_with_persistence`] to get a restorable app.
#[derive(uniffi::Record, Clone, Debug)]
pub struct PersistenceConfig {
    /// Directory holding the encrypted store (one file per identity).
    pub path: String,

    /// Passphrase protecting the store. **Set this.**
    ///
    /// When `Some`, a 256-bit AES key is derived from it and the store is
    /// genuinely confidential. When `None`, the key is instead derived from the
    /// app's **public** name: the store still detects tampering and is stable
    /// across restarts, but it offers **no confidentiality** — anyone who can
    /// read the database file and knows the app name can decrypt it. Only rely
    /// on the `None` fallback when the filesystem itself is your trust boundary.
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
    /// Create an app whose identity is provided by objects (foreign-language
    /// implementations), with restorable MLS/session
    /// state persisted under `persistence.path` (async).
    ///
    /// Persistence uses the provider's `export_identity` / `restore_identity`
    /// hooks; a provider whose `export_identity` returns `None` persists no identity, and
    /// one whose `export_identity` fails stops its session records from being updated.
    /// Restore the app's sessions after a restart with [`App::restore_sessions`].
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use std::sync::Arc;
    /// # use slim_bindings::*;
    /// # async fn demo(
    /// #     service: Arc<Service>,
    /// #     name: Arc<Name>,
    /// #     provider: Arc<dyn IdentityProvider>,
    /// #     verifier: Arc<dyn IdentityVerifier>,
    /// # ) -> Result<(), SlimError> {
    /// let app = service
    ///     .create_app_with_custom_identity_and_persistence_async(
    ///         name,
    ///         provider,
    ///         verifier,
    ///         Direction::Bidirectional,
    ///         PersistenceConfig {
    ///             path: "/var/lib/myapp/slim".into(),
    ///             passphrase: Some("a-strong-passphrase".into()),
    ///         },
    ///     )
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn create_app_with_custom_identity_and_persistence_async(
        &self,
        name: Arc<Name>,
        identity_provider: Arc<dyn crate::custom_identity::IdentityProvider>,
        identity_verifier: Arc<dyn crate::custom_identity::IdentityVerifier>,
        direction: Direction,
        persistence: PersistenceConfig,
    ) -> Result<Arc<App>, SlimError> {
        let slim_name: SlimName = name.as_ref().into();
        let (provider, verifier) =
            crate::custom_identity::into_core(identity_provider, identity_verifier);
        let service = self.inner.clone();
        let persistence_cfg: slim_persistence::PersistenceConfig = persistence.into();
        let handle = get_runtime().spawn(async move {
            create_app_from_identity(
                slim_name,
                provider,
                verifier,
                service,
                direction,
                Some(persistence_cfg),
            )
            .await
        });
        handle
            .await
            .map_err(|e| SlimError::InternalError {
                message: format!(
                    "create_app_with_custom_identity_and_persistence task failed: {e}"
                ),
            })?
            .map(Arc::new)
    }

    /// Blocking wrapper around
    /// [`Service::create_app_with_custom_identity_and_persistence_async`].
    pub fn create_app_with_custom_identity_and_persistence(
        &self,
        name: Arc<Name>,
        identity_provider: Arc<dyn crate::custom_identity::IdentityProvider>,
        identity_verifier: Arc<dyn crate::custom_identity::IdentityVerifier>,
        direction: Direction,
        persistence: PersistenceConfig,
    ) -> Result<Arc<App>, SlimError> {
        get_runtime().block_on(self.create_app_with_custom_identity_and_persistence_async(
            name,
            identity_provider,
            identity_verifier,
            direction,
            persistence,
        ))
    }

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
        let service = self.inner.clone();
        let persistence_cfg: slim_persistence::PersistenceConfig = persistence.into();
        // Route through the managed Tokio runtime so that internal tokio::spawn
        // calls (e.g. the message-processing loop spawned by bootstrap_app) have
        // a live reactor even when called from a UniFFI async context.
        let handle = get_runtime().spawn(async move {
            create_app_with_persistence_internal(
                slim_name,
                identity_provider_config,
                identity_verifier_config,
                service,
                direction,
                persistence_cfg,
            )
            .await
        });
        handle
            .await
            .map_err(|e| SlimError::InternalError {
                message: format!("create_app_with_persistence task failed: {e}"),
            })?
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

/// Provider-based app creation with persistence. Converts the configs and funnels
/// through the shared `create_app_from_identity`.
async fn create_app_with_persistence_internal(
    base_name: SlimName,
    identity_provider_config: IdentityProviderConfig,
    identity_verifier_config: IdentityVerifierConfig,
    service: Arc<SlimService>,
    direction: Direction,
    persistence: slim_persistence::PersistenceConfig,
) -> Result<App, SlimError> {
    let identity_provider: AuthProvider = identity_provider_config.try_into()?;
    let identity_verifier: AuthVerifier = identity_verifier_config.try_into()?;

    create_app_from_identity(
        base_name,
        identity_provider,
        identity_verifier,
        service,
        direction,
        Some(persistence),
    )
    .await
}
