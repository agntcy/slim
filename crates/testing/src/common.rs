// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use crate::utils::TEST_VALID_SECRET;
use slim_auth::shared_secret::SharedSecret;
use slim_config::component::{Component, id::ID};
use slim_config::grpc::server::ServerConfig as GrpcServerConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_datapath::api::ProtoName as Name;
use slim_service::ServiceError;
use slim_service::{Service, ServiceConfiguration, app::App};
use slim_session::{Notification, SessionError};
use slim_tracing::TracingConfiguration;
use std::net::TcpListener;
use std::time::Duration;
use tokio::sync::mpsc;

pub const DEFAULT_DATAPLANE_PORT: u16 = 46357;
pub const DEFAULT_SERVICE_ID: &str = "slim/0";

/// First port [`reserve_local_port`] hands out.
const TEST_PORTS_START: u16 = 20000;
/// How many ports it can hand out: up to 32767, just below the ephemeral
/// range of every OS (Linux 32768–60999, macOS and Windows 49152–65535).
const TEST_PORTS_COUNT: u16 = 12768;

/// Reserve a local TCP port for a test server.
///
/// The port is free when returned, and stays free until the test binds it:
/// it comes from below the ephemeral range, which the OS never hands out as
/// the local end of an outgoing connection. A port picked by binding port 0
/// comes from that range, so any client connecting anywhere could take it
/// before the server binds it -- which made tests fail with "address already
/// in use". Each process starts at its own offset, so test binaries running
/// side by side rarely try the same ports, and none is returned twice within
/// a process.
pub fn reserve_local_port() -> u16 {
    use std::sync::LazyLock;

    use parking_lot::Mutex;

    // Offset from the start of the range for the next port to try.
    static NEXT: LazyLock<Mutex<u16>> = LazyLock::new(|| {
        let spread = std::process::id().wrapping_mul(7919);
        Mutex::new((spread % u32::from(TEST_PORTS_COUNT)) as u16)
    });

    let mut next = NEXT.lock();
    for _ in 0..TEST_PORTS_COUNT {
        let port = TEST_PORTS_START + *next;
        *next = (*next + 1) % TEST_PORTS_COUNT;
        // Never handed out before (`next` only moves forward), so only
        // another process can hold it.
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!(
        "no free test port in {TEST_PORTS_START}..{}",
        TEST_PORTS_START + TEST_PORTS_COUNT
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_ports_are_distinct_and_below_the_ephemeral_range() {
        let ports: Vec<u16> = (0..200).map(|_| reserve_local_port()).collect();
        let distinct: std::collections::HashSet<_> = ports.iter().collect();
        assert_eq!(distinct.len(), ports.len());
        assert!(
            ports
                .iter()
                .all(|p| (TEST_PORTS_START..TEST_PORTS_START + TEST_PORTS_COUNT).contains(p))
        );
    }
}
