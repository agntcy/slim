// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};

use crate::api::proto::controller::proto::v1::controller_service_server::ControllerServiceServer;
use crate::api::proto::controlplane::proto::v1::control_plane_service_server::ControlPlaneServiceServer;
use crate::auth::DomainAuthenticator;
use crate::config::Config;
use crate::node_transport::DefaultNodeCommandHandler;
use crate::route_service::RouteService;
use crate::services::northbound::NorthboundApiService;
use crate::services::southbound::{SharedDrain, SouthboundApiService};
use crate::types::DEFAULT_SEGMENT;

pub struct ControlPlane {
    route_service: RouteService,
    drain_tx: drain::Signal,
    shared_drain: SharedDrain,
}

impl ControlPlane {
    pub async fn start(cfg: Config) -> Result<Self> {
        let db = crate::db::open(&cfg.database).await?;

        // In config mode, wipe all state on startup (config is source of truth).
        if cfg.topology.is_config_managed() {
            db.clear_all_state()
                .await
                .context("failed to clear all state")?;
            tracing::info!("topology mode: config-managed");
        } else {
            // In API mode, clear runtime state (nodes/links/routes) but keep
            // topology config (segments/segment_links) — nodes will re-register.
            db.clear_runtime_state()
                .await
                .context("failed to clear runtime state")?;
            tracing::info!("topology mode: API-managed");
        }

        let cmd_handler = DefaultNodeCommandHandler::new();
        let is_api_managed = cfg.topology.is_api_managed();
        let route_service = RouteService::new(
            db.clone(),
            cmd_handler.clone(),
            cfg.reconciler,
            cfg.topology.config,
        );

        // In API mode, ensure "default" segment exists, then load segment graphs from DB.
        if is_api_managed {
            if db.get_segment_by_name(DEFAULT_SEGMENT).await?.is_none() {
                db.create_segment(DEFAULT_SEGMENT)
                    .await
                    .context("failed to create default segment")?;
            }
            route_service
                .load_topology_from_db()
                .await
                .context("failed to load topology from DB")?;
        }

        // Build domain authenticator from config (Noop when no auth configured).
        let authenticator = match cfg.topology.auth {
            None => DomainAuthenticator::Noop,
            Some(auth_cfg) => Self::build_authenticator(auth_cfg, is_api_managed).await?,
        };

        // In API mode, restore DB-persisted secrets into the live authenticator.
        if is_api_managed && authenticator.is_shared_secret() {
            let domains = db.list_registration_secret_groups().await?;
            let mut restored = 0;
            for domain in &domains {
                let secret = match db.get_registration_secret(domain).await? {
                    Some(s) => s,
                    None => {
                        tracing::warn!(
                            "skipping domain '{domain}': listed but secret missing from DB"
                        );
                        continue;
                    }
                };
                if let Err(e) = authenticator.add_verifier(domain, &secret) {
                    tracing::warn!("skipping domain '{domain}': failed to build verifier: {e}");
                    continue;
                }
                restored += 1;
            }
            if restored > 0 {
                tracing::info!("restored {restored} domain secret(s) from DB");
            }
        }

        let nb_svc = NorthboundApiService::new(
            db.clone(),
            cmd_handler.clone(),
            route_service.clone(),
            authenticator.clone(),
        );

        let (drain_tx, drain_rx) = drain::channel();

        let shared_drain: SharedDrain =
            std::sync::Arc::new(parking_lot::Mutex::new(Some(drain_rx.clone())));
        let sb_svc = SouthboundApiService::new(
            db,
            cmd_handler,
            route_service.clone(),
            authenticator,
            shared_drain.clone(),
        );

        cfg.northbound
            .run_grpc_server(&[ControlPlaneServiceServer::new(nb_svc)], drain_rx.clone())
            .await
            .context("failed to start northbound server")?;

        cfg.southbound
            .run_grpc_server(&[ControllerServiceServer::new(sb_svc)], drain_rx)
            .await
            .context("failed to start southbound server")?;

        Ok(Self {
            route_service,
            drain_tx,
            shared_drain,
        })
    }

    pub async fn shutdown(self) {
        // Take the drain watch out of the service before signaling drain.
        // The service is behind tonic's Arc so it won't be dropped until all
        // handler tasks exit — but handler tasks need the drain signal to exit.
        // Taking it here breaks the circular dependency.
        self.shared_drain.lock().take();
        self.drain_tx.drain().await;
        self.route_service.shutdown().await;
    }

    async fn build_authenticator(
        cfg: crate::config::RegistrationAuthConfig,
        is_api_managed: bool,
    ) -> Result<DomainAuthenticator> {
        use crate::auth::SharedVerifiers;
        use crate::config::RegistrationAuthConfig;
        use std::collections::HashMap;

        match cfg {
            RegistrationAuthConfig::SharedSecret { secrets } => {
                if secrets.is_empty() && !is_api_managed {
                    return Err(anyhow::anyhow!(
                        "topology.registration_auth.shared_secret.secrets cannot be empty in config mode"
                    ));
                }
                let mut verifiers = HashMap::with_capacity(secrets.len());
                for (domain_name, secret) in secrets {
                    // id is not used on the verifier side — it only matters for
                    // token generation (provider). The verifier validates the HMAC
                    // using the secret alone.
                    let auth_config =
                        slim_config::auth::AuthConfig::SharedSecret { id: None, secret };
                    let (_, verifier_cfg) = auth_config.to_identity_configs(&domain_name);
                    let verifier = verifier_cfg.build_auth_verifier().map_err(|e| {
                        anyhow::anyhow!(
                            "failed to build auth verifier for domain '{domain_name}': {e}"
                        )
                    })?;
                    verifiers.insert(domain_name, verifier);
                }
                tracing::info!(
                    "registration auth: shared_secret for {} domain(s)",
                    verifiers.len()
                );
                let shared: SharedVerifiers =
                    std::sync::Arc::new(parking_lot::RwLock::new(verifiers));
                Ok(DomainAuthenticator::SharedSecret { verifiers: shared })
            }
            #[cfg(not(target_family = "windows"))]
            RegistrationAuthConfig::Spire { socket_path } => {
                use slim_config::auth::spire::SpireConfig;

                let spire_cfg = SpireConfig::new().with_socket_path(socket_path);
                let mut verifier = spire_cfg
                    .create_verifier()
                    .map_err(|e| anyhow::anyhow!("failed to build SPIRE verifier: {e}"))?;
                verifier
                    .initialize()
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to initialize SPIRE verifier: {e}"))?;
                let auth_verifier = slim_auth::auth_provider::AuthVerifier::Spire(verifier);
                tracing::info!("registration auth: spire (trust domain = domain name)");
                Ok(DomainAuthenticator::Spire {
                    verifier: Box::new(auth_verifier),
                })
            }
        }
    }
}
