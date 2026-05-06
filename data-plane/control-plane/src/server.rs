// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};

use crate::api::proto::controller::proto::v1::controller_service_server::ControllerServiceServer;
use crate::api::proto::controlplane::proto::v1::control_plane_service_server::ControlPlaneServiceServer;
use crate::config::Config;
use crate::node_transport::DefaultNodeCommandHandler;
use crate::route_service::RouteService;
use crate::services::northbound::NorthboundApiService;
use crate::services::southbound::{SharedDrain, SouthboundApiService};

pub struct ControlPlane {
    route_service: RouteService,
    drain_tx: drain::Signal,
    shared_drain: SharedDrain,
}

impl ControlPlane {
    pub async fn start(cfg: Config) -> Result<Self> {
        let db = crate::db::open(&cfg.database).await?;

        let cmd_handler = DefaultNodeCommandHandler::new();
        let route_service = RouteService::new(db.clone(), cmd_handler.clone(), cfg.reconciler);
        let nb_svc =
            NorthboundApiService::new(db.clone(), cmd_handler.clone(), route_service.clone());

        let (drain_tx, drain_rx) = drain::channel();

        let shared_drain: SharedDrain =
            std::sync::Arc::new(parking_lot::Mutex::new(Some(drain_rx.clone())));
        let sb_svc =
            SouthboundApiService::new(db, cmd_handler, route_service.clone(), shared_drain.clone());

        cfg.northbound
            .run_server(&[ControlPlaneServiceServer::new(nb_svc)], drain_rx.clone())
            .await
            .context("failed to start northbound server")?;

        cfg.southbound
            .run_server(&[ControllerServiceServer::new(sb_svc)], drain_rx)
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
}
