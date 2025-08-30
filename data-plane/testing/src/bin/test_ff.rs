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
use slim_service::{FireAndForgetConfiguration, ServiceConfiguration};
use slim_tracing::TracingConfiguration;

const DEFAULT_DATAPLANE_PORT: u16 = 46357;
const DEFAULT_SERVICE_ID: &str = "slim/0";

#[derive(Parser, Debug)]
pub struct Args {
    /// Runs the endpoint with MLS disabled.
    #[arg(
        short,
        long,
        value_name = "MSL_DISABLED",
        required = false,
        default_value_t = false
    )]
    mls_disabled: bool,

    /// Runs a sticky ff session.
    #[arg(
        short,
        long,
        value_name = "IS_STICKY",
        required = false,
        default_value_t = false
    )]
    is_sticky: bool,
}

impl Args {
    pub fn mls_disabled(&self) -> &bool {
        &self.mls_disabled
    }

    pub fn is_sticky(&self) -> &bool {
        &self.is_sticky
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

    app.subscribe(&name, Some(conn_id))
        .await
        .map_err(|_| format!("Failed to subscribe for participant {}", name))?;

    /* up to here */

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
                            let conn = msg.info.input_connection.unwrap();
                             if let Some(c) = msg.message.get_payload() {
                                 let blob = &c.blob;
                                 match String::from_utf8(blob.to_vec()) {
                                    Ok(val) => {
                                        if val != *"hello there" {
                                            // received corrupted message from the moderator
                                            continue;
                                        }

                                        // reply with the same payload to be sure that is was
                                        // decoded correctly in case of MLS
                                        let payload = val.into_bytes().to_vec();
                                        if app.publish_to(msg.info, &publisher, conn, payload, None, None)
                                            .await
                                            .is_err()
                                        {
                                            panic!("an error occurred sending publication from moderator");
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
    // get command line conf
    let args = Args::parse();
    let msl_enabled = !*args.mls_disabled();
    let is_sticky = *args.is_sticky();

    println!(
        "run test with MLS {} and sticky session {}",
        msl_enabled, is_sticky
    );

    // start slim node
    tokio::spawn(async move {
        let _ = run_slim_node().await;
    });

    // start clients
    let tot_clients = 3;
    let mut clients = vec![];

    for i in 0..tot_clients {
        let c = Name::from_strings(["org", "ns", "client"]).with_id(i);
        clients.push(c.clone());
        tokio::spawn(async move {
            let _ = run_client_task(c).await;
        });
    }

    // wait for all the processes to start
    tokio::time::sleep(tokio::time::Duration::from_millis(10000)).await;

    // start moderator
    let name = Name::from_strings(["org", "ns", "main"]).with_id(1);

    let client_config =
        GrpcClientConfig::with_endpoint(&format!("http://localhost:{}", DEFAULT_DATAPLANE_PORT))
            .with_tls_setting(TlsClientConfig::default().with_insecure(true));

    let mut config = create_service_configuration(client_config)?;

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

    app.subscribe(&name, Some(conn_id))
        .await
        .map_err(|_| format!("Failed to subscribe for participant {}", name))?;

    let info = app
        .create_session(
            slim_service::session::SessionConfig::FireAndForget(FireAndForgetConfiguration::new(
                Some(Duration::from_secs(1)),
                Some(10),
                is_sticky,
                msl_enabled,
            )),
            None,
        )
        .await
        .expect("error creating session");

    for c in &clients {
        // add routes
        app.set_route(c, conn_id)
            .await
            .expect("an error occurred while adding a route");
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

    // listen for messages
    let max_packets = 50;
    let recv_msgs = Arc::new(RwLock::new(HashMap::new()));
    let recv_msgs_clone = recv_msgs.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                None => {
                    break;
                }
                Some(msg_info) => match msg_info {
                    Ok(msg) => {
                        let sender = msg.message.get_source();
                        if let Some(c) = msg.message.get_payload() {
                            let p = &c.blob;
                            // check that we can read the message
                            let val = String::from_utf8(p.to_vec())
                                .expect("error while parsing received message");

                            if val != *"hello there" {
                                println!("received a corrupted reply");
                                continue;
                            }

                            // increase counter for the sender in the map
                            let mut lock = recv_msgs_clone.write();
                            match lock.get_mut(&sender) {
                                Some(x) => *x += 1,
                                None => {
                                    lock.insert(sender, 1);
                                }
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
    for i in 0..max_packets {
        println!("main: send message {}", i);

        if app
            .publish(
                info.clone(),
                &Name::from_strings(["org", "ns", "client"]),
                p.clone(),
                None,
                None,
            )
            .await
            .is_err()
        {
            panic!("an error occurred sending publication from moderator",);
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // the total number of packets received must be max_packets
    let mut sum = 0;
    // if sticky we must see a single sendere
    let mut found_sender = false;
    for (c, n) in recv_msgs.read().iter() {
        println!("received {} messages from {}", *n, c);
        sum += *n;
        if is_sticky && found_sender && *n != 0 {
            println!(
                "this is a sticky session but we got messages from multiple clients. test failed"
            );
            std::process::exit(1);
        }
        if *n != 0 {
            found_sender = true;
        }
    }

    if sum != max_packets {
        println!(
            "expected {} packets, received {}. test failed",
            sum, max_packets
        );
        std::process::exit(1);
    }

    println!("test succeeded");
    Ok(())
}
