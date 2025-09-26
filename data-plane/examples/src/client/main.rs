// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use tokio::time;
use tracing::info;

use slim::config;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::Name;

mod args;

#[tokio::main]
async fn main() {
    let args = args::Args::parse();

    let config_file = args.config();
    let local_name = args.local_name();
    let remote_name = args.remote_name();
    let message = args.message();

    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();

    info!(%config_file, %local_name, %remote_name, "starting client");

    let id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let mut svc = config.services.remove(&id).unwrap();

    let id = 0;
    let name = Name::from_strings(["org", "default", local_name]).with_id(id);
    let (app, mut rx) = svc
        .create_app(
            &name,
            SharedSecret::new(local_name, "group"),
            SharedSecret::new(local_name, "group"),
        )
        .await
        .expect("failed to create app");

    svc.run().await.unwrap();

    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .unwrap();

    let local_app_name = Name::from_strings(["org", "default", local_name]).with_id(id);
    app.subscribe(&local_app_name, Some(conn_id)).await.unwrap();

    let route = Name::from_strings(["org", "default", remote_name]).with_id(id);
    info!("allowing messages to remote app: {:?}", route);
    app.set_route(&route, conn_id).await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    info!("CLIENT: Entering message receive loop");
    loop {
        tokio::select! {
            _ = slim_signal::shutdown() => {
                info!("Received shutdown signal");
                break;
            }
            next = rx.recv() => {
                if next.is_none() {
                    info!("end of stream");
                    break;
                }

                info!("CLIENT: received something from rx.recv()");
                info!("CLIENT: message details: {:?}", next);

                let session_msg = match next.unwrap() {
                    Ok(msg) => {
                        info!("CLIENT: successfully parsed session message");
                        msg
                    },
                    Err(e) => {
                        info!("CLIENT: received error message: {:?}", e);
                        continue;
                    }
                };

                let publisher = session_msg.message.get_slim_header().get_source();
                let msg_id = session_msg.message.get_id();
                info!("CLIENT: message from {:?}, id: {}", publisher, msg_id);

                if let Some(c) = session_msg.message.get_payload() {
                    let blob = &c.blob;
                    info!("CLIENT: message has payload of {} bytes", blob.len());
                    match String::from_utf8(blob.to_vec()) {
                        Ok(text) => {
                            info!("received message: {}", text);

                            if message.is_some() {
                                let response = format!("hello from the {}", local_name);
                                info!("CLIENT: sending response: {}", response);
                                let _ = app.publish(session_msg.info, &route, response.into(), None, None).await;
                            }
                        }
                        Err(e) => {
                            info!("received encrypted/binary message: {} bytes, error: {}", blob.len(), e);
                        }
                    }
                } else {
                    info!("received message without payload (possibly invitation)");
                }
            }
        }
    }

    info!("client shutting down");

    let signal = svc.signal();

    match time::timeout(config.runtime.drain_timeout(), signal.drain()).await {
        Ok(()) => {}
        Err(_) => panic!("timeout waiting for drain for service"),
    }
}
