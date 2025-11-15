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
use slim_service::{ServiceConfiguration, SlimHeaderFlags};
use slim_session::{Notification, SessionConfig};
use slim_testing::utils::TEST_VALID_SECRET;
use slim_tracing::TracingConfiguration;

const DEFAULT_DATAPLANE_PORT: u16 = 46357;
const DEFAULT_SERVICE_ID: &str = "slim/0";

#[derive(Parser, Debug)]
pub struct Args {
    /// Runs the endpoint with MLS disabled.
    #[arg(
        short,
        long,
        value_name = "MLS_DISABLED",
        required = false,
        default_value_t = false
    )]
    mls_disabled: bool,
}

impl Args {
    pub fn mls_disabled(&self) -> &bool {
        &self.mls_disabled
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

async fn run_participant_task(name: Name) -> Result<(), String> {
    println!("Participant {:?} task starting...", name);

    let dataplane_client_config =
        GrpcClientConfig::with_endpoint(&format!("http://localhost:{}", DEFAULT_DATAPLANE_PORT))
            .with_tls_setting(TlsClientConfig::default().with_insecure(true));

    let mut config = create_service_configuration(dataplane_client_config)?;

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

    let moderator = Name::from_strings(["org", "ns", "moderator"]).with_id(1);
    let channel_name = Name::from_strings(["channel", "channel", "channel"]);

    let name_clone = name.clone();
    let moderator_clone = moderator.clone();
    let channel_name_clone = channel_name.clone();
    loop {
        tokio::select! {
            msg_result = rx.recv() => {
                match msg_result {
                    None => { println!("Participant {}: end of stream", name_clone); break; }
                    Some(res) => match res {
                        Ok(notification) => match notification {
                            Notification::NewSession(session_ctx) => {
                                let session_moderator_clone = moderator_clone.clone();
                                let session_channel_name_clone = channel_name_clone.clone();
                                let session_name = name_clone.clone();
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
                                                    let msg_id = msg.get_id();
                                                    let blob = &publish.get_payload().as_application_payload().unwrap().blob;
                                                    if let Ok(val) = String::from_utf8(blob.to_vec()) {
                                                        if publisher == session_moderator_clone {
                                                            if val != *"hello there" { continue; }
                                                            let payload = msg_id.to_ne_bytes().to_vec();
                                                            let flags = SlimHeaderFlags::new(10, None, None, None, None);
                                                            if let Some(session_arc) = weak.upgrade() &&
                                                                session_arc.publish_with_flags(&session_channel_name_clone, flags, payload, None, None).await.is_err() {
                                                                panic!("an error occurred sending publication from moderator");
                                                            }
                                                        }
                                                    } else { println!("Participant {}: error parsing message", session_name); }
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

    if mls_enabled {
        println!("start test with msl enabled");
    } else {
        println!("start test with msl disabled");
    }
    // start slim node
    tokio::spawn(async move {
        let _ = run_slim_node().await;
    });

    // start clients
    let tot_participants = 5;
    let mut participants = vec![];

    for i in 0..tot_participants {
        let p = Name::from_strings(["org", "ns", &format!("t{}", i)]);
        participants.push(p.clone());
        tokio::spawn(async move {
            let _ = run_participant_task(p.with_id(1)).await;
        });
    }

    // wait for all the processes to start
    tokio::time::sleep(tokio::time::Duration::from_millis(10000)).await;

    // start moderator
    let name = Name::from_strings(["org", "ns", "moderator"]).with_id(1);
    let channel_name = Name::from_strings(["channel", "channel", "channel"]);

    let dataplane_client_config =
        GrpcClientConfig::with_endpoint(&format!("http://localhost:{}", DEFAULT_DATAPLANE_PORT))
            .with_tls_setting(TlsClientConfig::default().with_insecure(true));

    let mut config = create_service_configuration(dataplane_client_config)?;

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
        session_type: slim_datapath::api::ProtoSessionType::Multicast,
        max_retries: Some(10),
        interval: Some(Duration::from_secs(1)),
        mls_enabled,
        initiator: true,
        metadata: HashMap::new(),
    };
    let (session_ctx, completion_handle) = app
        .create_session(conf, channel_name.clone(), None)
        .await
        .expect("error creating session");

    // Await the completion of the session establishment
    completion_handle.await.expect("error establishing session");

    for c in &participants {
        // add routes
        app.set_route(c, conn_id)
            .await
            .expect("an error occurred while adding a route");
    }

    // invite N-1 participants
    for c in participants.iter().take(tot_participants - 1) {
        println!("Invite participant {}", c);
        session_ctx
            .session_arc()
            .unwrap()
            .invite_participant(c)
            .await
            .expect("error sending invite message");
    }

    // listen for messages
    let max_packets = 100;
    let recv_msgs = Arc::new(RwLock::new(vec![0; max_packets]));
    let recv_msgs_clone = recv_msgs.clone();

    // Clone the Arc to session for later use
    let session_arc = session_ctx.session_arc().unwrap();

    session_ctx.spawn_receiver(move |mut rx, _weak| async move {
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
                            let blob =
                                &publish.get_payload().as_application_payload().unwrap().blob;
                            let _ = String::from_utf8(blob.to_vec())
                                .expect("error while parsing received message");
                            if blob.len() >= 4 {
                                let bytes_array: [u8; 4] = blob[0..4].try_into().unwrap();
                                let id = u32::from_ne_bytes(bytes_array) as usize;
                                println!("recv msg {} from {}", id, msg.get_source());
                                let mut lock = recv_msgs_clone.write();
                                if id < lock.len() {
                                    lock[id] += 1;
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

    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

    let msg_payload_str = "hello there";
    let p = msg_payload_str.as_bytes().to_vec();
    let mut to_add = tot_participants - 1;
    let mut to_remove = 0;
    for i in 1..max_packets {
        println!("moderator: send message {}", i);

        // set fanout > 1 to send the message in broadcast
        let flags = SlimHeaderFlags::new(10, None, None, None, None);

        if session_arc
            .publish_with_flags(&channel_name, flags, p.clone(), None, None)
            .await
            .is_err()
        {
            panic!("an error occurred sending publication from moderator",);
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        if i % 10 == 0 {
            println!(
                "remove {} and add {}",
                &participants[to_remove], &participants[to_add]
            );

            let _ = session_arc
                .remove_participant(&participants[to_remove])
                .await;
            let _ = session_arc.invite_participant(&participants[to_add]).await;
            to_remove = (to_remove + 1) % tot_participants;
            to_add = (to_add + 1) % tot_participants;

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    for i in 1..max_packets {
        let lock = recv_msgs.read();
        if lock[i] != (tot_participants - 1) {
            println!(
                "error for message id {}. expected {} packets, received {}. exit with error",
                i,
                (tot_participants - 1),
                lock[i]
            );
            std::process::exit(1);
        }
    }

    // close session
    let handle = session_arc.close().expect("error closing session");
    handle.await.expect("error waiting the handler");
    println!("test succeeded");
    Ok(())
}
