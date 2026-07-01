// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};

use crate::api::proto::controller::proto::v1::controller_service_server::ControllerServiceServer;
use crate::api::proto::controlplane::proto::v1::control_plane_service_server::ControlPlaneServiceServer;
use crate::auth::GroupAuthenticator;
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
            cfg.topology,
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
        let nb_svc =
            NorthboundApiService::new(db.clone(), cmd_handler.clone(), route_service.clone());

        // Build group authenticator from config (Noop when no auth configured).
        let authenticator = match cfg.registration_auth {
            None => GroupAuthenticator::Noop,
            Some(auth_cfg) => Self::build_authenticator(auth_cfg)?,
        };

        let (drain_tx, drain_rx) = drain::channel();

        let shared_drain: SharedDrain =
            std::sync::Arc::new(parking_lot::Mutex::new(Some(drain_rx.clone())));
        let sb_svc = SouthboundApiService::new(
            db,
            cmd_handler,
            route_service.clone(),
            shared_drain.clone(),
            authenticator,
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

    fn build_authenticator(
        cfg: crate::config::RegistrationAuthConfig,
    ) -> Result<GroupAuthenticator> {
        use crate::config::RegistrationAuthConfig;
        use std::collections::HashMap;

        match cfg {
            RegistrationAuthConfig::SharedSecret { groups } => {
                if groups.is_empty() {
                    return Err(anyhow::anyhow!(
                        "registration_auth.shared_secret.groups cannot be empty"
                    ));
                }
                let mut verifiers = HashMap::with_capacity(groups.len());
                for (group_name, auth_config) in groups {
                    let (_, verifier_cfg) = auth_config.to_identity_configs(&group_name);
                    let verifier = verifier_cfg.build_auth_verifier().map_err(|e| {
                        anyhow::anyhow!(
                            "failed to build auth verifier for group '{group_name}': {e}"
                        )
                    })?;
                    verifiers.insert(group_name, verifier);
                }
                tracing::info!(
                    "registration auth: shared_secret for {} group(s)",
                    verifiers.len()
                );
                Ok(GroupAuthenticator::SharedSecret { verifiers })
            }
            #[cfg(not(target_family = "windows"))]
            RegistrationAuthConfig::Spire { socket_path } => {
                tracing::warn!(
                    "SPIRE registration auth not yet implemented (socket: {socket_path}); falling back to Noop"
                );
                Ok(GroupAuthenticator::Noop)
            }
        }
    }
}
