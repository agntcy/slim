// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use slim::runtime::RuntimeConfiguration;
use slim_auth::shared_secret::SharedSecret;
use slim_config::component::{Component, id::ID};
use slim_config::grpc::client::ClientConfig as GrpcClientConfig;
use slim_config::grpc::server::ServerConfig as GrpcServerConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_datapath::messages::{Agent, AgentType};
use slim_service::{ServiceConfiguration, SlimHeaderFlags, StreamingConfiguration};
use slim_tracing::TracingConfiguration;

const DEFAULT_PUBSUB_PORT: u16 = 46357;
const DEFAULT_SERVICE_ID: &str = "slim/0";

async fn run_slim_node() -> Result<(), String> {
    println!("Server task starting...");

    let pubsub_server_config =
        GrpcServerConfig::with_endpoint(&format!("0.0.0.0:{}", DEFAULT_PUBSUB_PORT))
            .with_tls_settings(TlsServerConfig::default().with_insecure(true));

    let service_config = ServiceConfiguration::new().with_server(vec![pubsub_server_config]);

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

async fn run_participant_task(name: Agent) -> Result<(), String> {
    println!("Participant {:?} task starting...", name);

    let pubsub_client_config =
        GrpcClientConfig::with_endpoint(&format!("http://localhost:{}", DEFAULT_PUBSUB_PORT))
            .with_tls_setting(TlsClientConfig::default().with_insecure(true));

    let mut config = create_service_configuration(pubsub_client_config)?;

    let svc_id = ID::new_with_str(DEFAULT_SERVICE_ID).unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    let (app, mut rx) = svc
        .create_app(
            &name,
            SharedSecret::new(&name.to_string(), "group"),
            SharedSecret::new(&name.to_string(), "group"),
        )
        .await
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

    app.subscribe(name.agent_type(), name.agent_id_option(), Some(conn_id))
        .await
        .map_err(|_| format!("Failed to subscribe for participant {}", name))?;

    let moderator = Agent::from_strings("org", "ns", "moderator", 1);
    let channel_name = AgentType::from_strings("channel", "channel", "channel");

    loop {
        tokio::select! {
            msg_result = rx.recv() => {
                match msg_result {
                    None => {
                        println!("Participant {}: end of stream", name);
                        break;
                    }
                    Some(msg_info) => match msg_info {
                        Ok(msg) => {
                            let publisher = msg.message.get_slim_header().get_source();
                            let msg_id = msg.message.get_id();
                             if let Some(c) = msg.message.get_payload() {
                                 let blob = &c.blob;
                                 match String::from_utf8(blob.to_vec()) {
                                     Ok(val) => {
                                         if publisher == moderator {
                                             if val != *"hello there" {
                                                 // received corrupted message from the moderator
                                                 continue;
                                             }

                                             // put the received msg id in the payload
                                             let payload = msg_id.to_ne_bytes().to_vec();
                                             let flags = SlimHeaderFlags::new(10, None, None, None, None);
                                             if app
                                                 .publish_with_flags(msg.info, &channel_name, None, flags, payload)
                                                 .await
                                                 .is_err()
                                             {
                                                 panic!("an error occurred sending publication from moderator");
                                             }
                                         }
                                     },
                                     Err(e) => {
                                         println!("Participant {}: error parsing message: {}", name, e);
                                         continue;
                                     }
                                 }
                             }
                        }
                        Err(e) => {
                            println!("Participant {} received error message: {:?}", name, e);
                        }
                    },
                }
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // start slim node
    tokio::spawn(async move {
        let _ = run_slim_node().await;
    });

    // start clients
    let tot_participants = 5;
    let mut participants = vec![];

    for i in 0..tot_participants {
        let p = Agent::from_strings("org", "ns", &format!("t{}", i), 1);
        participants.push(p.clone());
        tokio::spawn(async move {
            let _ = run_participant_task(p).await;
        });
    }

    // wait for all the processes to start
    tokio::time::sleep(tokio::time::Duration::from_millis(10000)).await;

    // start moderator
    let name = Agent::from_strings("org", "ns", "moderator", 1);
    let channel_name = AgentType::from_strings("channel", "channel", "channel");

    let pubsub_client_config =
        GrpcClientConfig::with_endpoint(&format!("http://localhost:{}", DEFAULT_PUBSUB_PORT))
            .with_tls_setting(TlsClientConfig::default().with_insecure(true));

    let mut config = create_service_configuration(pubsub_client_config)?;

    let svc_id = ID::new_with_str(DEFAULT_SERVICE_ID).unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    let (app, mut rx) = svc
        .create_app(
            &name,
            SharedSecret::new(&name.to_string(), "group"),
            SharedSecret::new(&name.to_string(), "group"),
        )
        .await
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

    app.subscribe(name.agent_type(), name.agent_id_option(), Some(conn_id))
        .await
        .map_err(|_| format!("Failed to subscribe for participant {}", name))?;

    let info = app
        .create_session(
            slim_service::session::SessionConfig::Streaming(StreamingConfiguration::new(
                slim_service::session::SessionDirection::Bidirectional,
                Some(channel_name.clone()),
                true,
                Some(10),
                Some(Duration::from_secs(1)),
                true,
            )),
            None,
        )
        .await
        .expect("error creating session");

    for c in &participants {
        // add routes
        app.set_route(c.agent_type(), None, conn_id)
            .await
            .expect("an error occurred while adding a route");
    }

    // invite N-1 participants
    for c in participants.iter().take(tot_participants - 1) {
        println!("Invite participant {}", c);
        app.invite_participant(c.agent_type(), info.clone())
            .await
            .expect("error sending invite message");
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

    // listen for messages
    let max_packets = 100;
    let recv_msgs = Arc::new(RwLock::new(vec![0; max_packets]));
    let recv_msgs_clone = recv_msgs.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                None => {
                    break;
                }
                Some(msg_info) => match msg_info {
                    Ok(msg) => {
                        if let Some(c) = msg.message.get_payload() {
                            let p = &c.blob;
                            // check that we can read the message
                            let _ = String::from_utf8(p.to_vec())
                                .expect("error while parsing received message");

                            let bytes_array: [u8; 4] =
                                <Vec<u8> as Clone>::clone(&c.blob).try_into().unwrap();
                            let id = u32::from_ne_bytes(bytes_array) as usize;
                            {
                                println!("recv msg {} from {}", id, msg.message.get_source());
                                let mut lock = recv_msgs_clone.write();
                                lock[id] += 1;
                            }
                        };
                    }
                    Err(e) => {
                        println!("received an error message {:?}", e);
                    }
                },
            }
        }
    });

    let msg_payload_str = "hello there";
    let p = msg_payload_str.as_bytes().to_vec();
    let mut to_add = tot_participants - 1;
    let mut to_remove = 0;
    for i in 0..max_packets {
        println!("moderator: send message {}", i);

        // set fanout > 1 to send the message in broadcast
        let flags = SlimHeaderFlags::new(10, None, None, None, None);

        if app
            .publish_with_flags(info.clone(), &channel_name, None, flags, p.clone())
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

            let _ = app
                .remove_participant(&participants[to_remove], info.clone())
                .await;
            let _ = app
                .invite_participant(participants[to_add].agent_type(), info.clone())
                .await;
            to_remove = (to_remove + 1) % tot_participants;
            to_add = (to_add + 1) % tot_participants;

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    for i in 0..max_packets {
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
    println!("test succeeded");
    Ok(())
}
