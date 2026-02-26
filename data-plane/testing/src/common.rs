// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::utils::TEST_VALID_SECRET;
use slim_auth::shared_secret::SharedSecret;
use slim_config::component::{Component, id::ID};
use slim_config::grpc::server::ServerConfig as GrpcServerConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_datapath::messages::Name;
use slim_service::ServiceError;
use slim_service::{Service, ServiceConfiguration, app::App};
use slim_session::{Notification, SessionError};
use slim_tracing::TracingConfiguration;
use std::net::TcpListener;
use std::time::Duration;
use tokio::sync::mpsc;

pub const DEFAULT_DATAPLANE_PORT: u16 = 46357;
pub const DEFAULT_SERVICE_ID: &str = "slim/0";

/// Reserve a local TCP port for a test server.
pub fn reserve_local_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test port");
    let port = listener
        .local_addr()
        .expect("failed to read test port")
        .port();
    drop(listener);
    port
}

/// Runs a SLIM node server that listens on the provided dataplane port.
/// The server will run until it receives a shutdown signal or times out after 5 minutes.
pub async fn run_slim_node(port: u16) -> Result<(), ServiceError> {
    println!("Server task starting...");

    let dataplane_server_config = GrpcServerConfig::with_endpoint(&format!("0.0.0.0:{}", port))
        .with_tls_settings(TlsServerConfig::default().with_insecure(true));

    let service_config =
        ServiceConfiguration::new().with_dataplane_server(vec![dataplane_server_config]);

    let svc_id = ID::new_with_str(DEFAULT_SERVICE_ID).unwrap();
    let mut service = service_config.build_server(svc_id.clone())?;

    let tracing = TracingConfiguration::default();
    let _guard = tracing.setup_tracing_subscriber();

    println!("Starting service: {}", svc_id);
    service.start().await?;

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("Server received shutdown signal");
        }
        _ = tokio::time::sleep(Duration::from_secs(300)) => {
            println!("Server timeout after 5 minutes");
        }
    }

    Ok(())
}

/// Creates an app for a given service and subscribes it to the specified name.
/// Returns the app handle, receiver channel, connection ID, and the service.
pub async fn create_and_subscribe_app(
    svc: Service,
    name: &Name,
) -> Result<
    (
        App<SharedSecret, SharedSecret>,
        mpsc::Receiver<Result<Notification, SessionError>>,
        u64,
        Service,
    ),
    ServiceError,
> {
    let (app, rx) = svc.create_app(
        name,
        SharedSecret::new(&name.to_string(), TEST_VALID_SECRET)?,
        SharedSecret::new(&name.to_string(), TEST_VALID_SECRET)?,
    )?;

    svc.run().await?;

    let conn_id = svc
        .get_connection_id(&svc.config().dataplane_clients()[0].endpoint)
        .ok_or(ServiceError::ConnectionNotFoundForEndpoint(
            svc.config().dataplane_clients()[0].endpoint.clone(),
        ))?;

    app.subscribe(name, Some(conn_id)).await?;

    Ok((app, rx, conn_id, svc))
}
