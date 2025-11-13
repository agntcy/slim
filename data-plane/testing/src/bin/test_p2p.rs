// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use parking_lot::RwLock;

use slim::runtime::RuntimeConfiguration;
use slim_auth::shared_secret::SharedSecret;
use slim_config::component::{Component, id::ID};
use slim_config::grpc::client::ClientConfig as GrpcClientConfig;
use slim_config::grpc::server::ServerConfig as GrpcServerConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_datapath::messages::Name;
use slim_service::ServiceConfiguration;
use slim_session::{Notification, SessionConfig};
use slim_testing::utils::TEST_VALID_SECRET;
use slim_tracing::TracingConfiguration;

const DEFAULT_DATAPLANE_PORT: u16 = 46357;
const DEFAULT_SERVICE_ID: &str = "slim/0";

#[derive(Parser, Debug)]
pub struct Args {
    /// Runs the session with MLS disabled.
    #[arg(
        short,
        long,
        value_name = "MLS_DISABLED",
        required = false,
        default_value_t = false
    )]
    mls_disabled: bool,

    /// Do not run SLIM node in background.
    #[arg(
        short,
        long,
        value_name = "SLIM_DISABLED",
        required = false,
        default_value_t = false
    )]
    slim_disabled: bool,

    /// Apps to run.
    #[arg(
        short,
        long,
        value_name = "APPS",
        required = false,
        default_value_t = 3
    )]
    apps: u32,
}

impl Args {
    pub fn mls_disabled(&self) -> &bool {
        &self.mls_disabled
    }

    pub fn slim_disabled(&self) -> &bool {
        &self.slim_disabled
    }

    pub fn apps(&self) -> &u32 {
        &self.apps
    }
}

async fn run_slim_node() -> Result<(), String> {
    println!("Server task starting...");

    let dataplane_server_config =
        GrpcServerConfig::with_endpoint(&format!("0.0.0.0:{}", DEFAULT_DATAPLANE_PORT))
            .with_tls_settings(TlsServerConfig::default().with_insecure(true));

    let service_config = ServiceConfiguration::new().with_server(vec![dataplane_server_config]);

    let svc_id = ID::new_with_str(DEFAULT_SERVICE_ID).unwrap();
    let service = service_config
        .build_server(svc_id.clone())
        .map_err(|e| format!("Failed to build server: {}", e))?;

    let mut services = HashMap::new();
    services.insert(svc_id, service);

    let mut server_config = slim::config::ConfigResult {
        tracing: TracingConfiguration::default(),
        runtime: RuntimeConfiguration::default(),
        services,
    };

    let _guard = server_config.tracing.setup_tracing_subscriber();

    for service in server_config.services.iter_mut() {
        println!("Starting service: {}", service.0);
        service
            .1
            .start()
            .await
            .map_err(|e| format!("Failed to start service {}: {}", service.0, e))?;
    }

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

fn create_service_configuration(
    client_config: GrpcClientConfig,
) -> Result<slim::config::ConfigResult, String> {
    let service_config = ServiceConfiguration::new().with_client(vec![client_config]);

    let svc_id = ID::new_with_str(DEFAULT_SERVICE_ID).unwrap();
    let service = service_config
        .build_server(svc_id.clone())
        .map_err(|e| format!("Failed to build service: {}", e))?;

    let mut services = HashMap::new();
    services.insert(svc_id, service);

    let config = slim::config::ConfigResult {
        tracing: TracingConfiguration::default(),
        runtime: RuntimeConfiguration::default(),
        services,
    };

    Ok(config)
}

async fn run_client_task(name: Name) -> Result<(), String> {
    /* this is the same */
    println!("client {:?} task starting...", name);

    let client_config =
        GrpcClientConfig::with_endpoint(&format!("http://localhost:{}", DEFAULT_DATAPLANE_PORT))
            .with_tls_setting(TlsClientConfig::default().with_insecure(true));

    let mut config = create_service_configuration(client_config)?;

    let svc_id = ID::new_with_str(DEFAULT_SERVICE_ID).unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    let (app, mut rx) = svc
        .create_app(
            &name,
            SharedSecret::new(&name.to_string(), TEST_VALID_SECRET),
            SharedSecret::new(&name.to_string(), TEST_VALID_SECRET),
        )
        .map_err(|_| format!("Failed to create participant {}", name))?;

    svc.run()
        .await
        .map_err(|_| format!("Failed to run participant {}", name))?;

    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .ok_or(format!(
            "Failed to get connection id for participant {}",
            name,
        ))?;

    app.subscribe(&name, Some(conn_id))
        .await
        .map_err(|_| format!("Failed to subscribe for participant {}", name))?;

    let name_clone = name.clone();
    loop {
        tokio::select! {
            msg_result = rx.recv() => {
                match msg_result {
                    None => { println!("Participant {}: end of stream", name_clone); break; }
                    Some(res) => match res {
                        Ok(notification) => match notification {
                            Notification::NewSession(session_ctx) => {
                                println!("create new session on client {}", name_clone);
                                let name_clone_session = name_clone.clone();
                                session_ctx.spawn_receiver(move |mut rx, weak| async move {
                                    loop{
                                        match rx.recv().await {
                                            None => {
                                                println!("Session receiver: end of stream");
                                                break;
                                            }
                                            Some(Ok(msg)) => {
                                                if let Some(slim_datapath::api::ProtoPublishType(publish)) = msg.message_type.as_ref() {
                                                    let publisher = msg.get_slim_header().get_source();
                                                    let conn = msg.get_slim_header().recv_from.unwrap_or(conn_id);
                                                    let blob = &publish.get_payload().as_application_payload().unwrap().blob;
                                                    match String::from_utf8(blob.to_vec()) {
                                                        Ok(val) => {
                                                            if val != *"hello there" { continue; }
                                                            if let Some(session_arc) = weak.upgrade() {
                                                                let payload = val.into_bytes();
                                                                println!("received message {} on app {}", msg.get_session_header().get_message_id(), name_clone_session);
                                                                if session_arc.publish_to(&publisher, conn, payload, None, None).await.is_err() {
                                                                    panic!("an error occurred sending publication from moderator");
                                                                }
                                                            }
                                                        }
                                                        Err(e) => { println!("Participant {}: error parsing message: {}", name_clone_session, e); continue; }
                                                    }
                                                }
                                            }
                                            Some(Err(e)) => {
                                                println!("Session receiver: error {:?}", e);
                                                break;
                                            }
                                        }
                                    }
                                });
                            }
                            _ => {
                                println!("Unexpected notification type");
                                continue;
                            }
                        }
                        Err(e) => { println!("Participant {} received error message: {:?}", name, e); }
                    }
                }
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // get command line conf
    let args = Args::parse();
    let mls_enabled = !*args.mls_disabled();
    let slim_disabled = *args.slim_disabled();
    let apps = *args.apps();

    println!(
        "run test with MLS = {} number of apps = {}, SLIM on = {}",
        mls_enabled, apps, !slim_disabled,
    );

    // start slim node
    if !slim_disabled {
        tokio::spawn(async move {
            let _ = run_slim_node().await;
        });
    }

    // start clients
    let tot_clients = apps;
    let mut clients = vec![];

    for i in 0..tot_clients {
        let c = Name::from_strings(["org", "ns", "client"]).with_id(i.into());
        clients.push(c.clone());
        tokio::spawn(async move {
            let _ = run_client_task(c).await;
        });
    }

    // start moderator
    let name = Name::from_strings(["org", "ns", "main"]).with_id(1);

    let client_config =
        GrpcClientConfig::with_endpoint(&format!("http://localhost:{}", DEFAULT_DATAPLANE_PORT))
            .with_tls_setting(TlsClientConfig::default().with_insecure(true));

    let mut config = create_service_configuration(client_config)?;

    let svc_id = ID::new_with_str(DEFAULT_SERVICE_ID).unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    let (app, _rx) = svc
        .create_app(
            &name,
            SharedSecret::new(&name.to_string(), TEST_VALID_SECRET),
            SharedSecret::new(&name.to_string(), TEST_VALID_SECRET),
        )
        .map_err(|_| format!("Failed to create moderator {}", name))?;

    svc.run()
        .await
        .map_err(|_| format!("Failed to run participant {}", name))?;

    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .ok_or(format!(
            "Failed to get connection id for participant {}",
            name,
        ))?;

    app.subscribe(&name, Some(conn_id))
        .await
        .map_err(|_| format!("Failed to subscribe for participant {}", name))?;

    let conf = SessionConfig {
        session_type: slim_datapath::api::ProtoSessionType::PointToPoint,
        max_retries: Some(10),
        interval: Some(Duration::from_secs(1)),
        mls_enabled,
        initiator: true,
        metadata: HashMap::new(),
    };

    for c in &clients {
        // add routes
        app.set_route(c, conn_id)
            .await
            .expect("an error occurred while adding a route");
    }

    let (session_ctx, completion_handle) = app
        .create_session(conf, Name::from_strings(["org", "ns", "client"]), None)
        .await
        .expect("error creating session");

    // Wait for session to be established
    completion_handle.await.expect("error establishing session");

    // listen for messages
    let max_packets = 50;
    let recv_msgs = Arc::new(RwLock::new(HashMap::new()));
    let recv_msgs_clone = recv_msgs.clone();

    // Clone the Arc to session for later use
    let session_arc = session_ctx.session_arc().unwrap();

    // Create a channel to signal when all messages are received
    let (all_received_tx, all_received_rx) = tokio::sync::oneshot::channel();

    session_ctx.spawn_receiver(move |mut rx, _weak| async move {
        let mut all_received_tx = Some(all_received_tx);
        loop {
            match rx.recv().await {
                None => {
                    println!("end of stream");
                    break;
                }
                Some(message) => match message {
                    Ok(msg) => {
                        if let Some(slim_datapath::api::ProtoPublishType(publish)) =
                            msg.message_type.as_ref()
                        {
                            let sender = msg.get_source();
                            let p = &publish.get_payload().as_application_payload().unwrap().blob;
                            let val = String::from_utf8(p.to_vec())
                                .expect("error while parsing received message");
                            if val != *"hello there" {
                                println!("received a corrupted reply");
                                continue;
                            }
                            recv_msgs_clone
                                .write()
                                .entry(sender)
                                .and_modify(|v| *v += 1)
                                .or_insert(1);

                            // Check if we've received all expected messages
                            let total: u32 = recv_msgs_clone.read().values().sum();
                            if total >= max_packets {
                                println!(
                                    "Received all {} messages, signaling completion",
                                    max_packets
                                );
                                if let Some(tx) = all_received_tx.take() {
                                    let _ = tx.send(());
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!("error receiving message {}", e);
                        continue;
                    }
                },
            }
        }
    });

    let msg_payload_str = "hello there";
    let p = msg_payload_str.as_bytes().to_vec();
    let mut completion_handlers = vec![];
    for i in 0..max_packets {
        println!("main: send message {}", i);

        let completion_handler = session_arc
            .publish(
                &Name::from_strings(["org", "ns", "client"]),
                p.clone(),
                None,
                None,
            )
            .await
            .expect("an error occurred sending publication from moderator");

        completion_handlers.push(completion_handler);
    }

    // wait for all messages to be sent
    futures::future::try_join_all(completion_handlers)
        .await
        .expect("an error occurred waiting for publication completion from moderator");

    // Wait for all messages to be received with a timeout
    tokio::select! {
        _ = all_received_rx => {
            println!("All messages received successfully");
        }
        _ = tokio::time::sleep(Duration::from_secs(30)) => {
            println!("Timeout waiting for all messages to be received");
        }
    }

    // the total number of packets received must be max_packets
    let mut sum = 0;
    // if unicast we must see a single sendere
    let mut found_sender = false;
    for (c, n) in recv_msgs.read().iter() {
        sum += *n;
        if found_sender && *n != 0 {
            println!(
                "this is a unicast session but we got messages from multiple clients. test failed"
            );
            std::process::exit(1);
        }
        if *n != 0 {
            found_sender = true;
        }
        println!("received {} messages from {}", n, c);
    }

    if sum != max_packets {
        println!(
            "expected {} packets, received {}. test failed",
            max_packets, sum
        );
        std::process::exit(1);
    }

    println!("test succeeded");
    Ok(())
}
