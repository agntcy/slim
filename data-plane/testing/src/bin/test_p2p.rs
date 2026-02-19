// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use parking_lot::RwLock;

use slim_datapath::messages::Name;
use slim_service::ServiceError;
use slim_session::{Notification, SessionConfig};
use slim_testing::build_client_service;
use slim_testing::common::{
    DEFAULT_SERVICE_ID, create_and_subscribe_app, reserve_local_port, run_slim_node,
};

#[derive(Parser, Debug)]
pub struct Args {
    /// Runs the session with MLS disabled.
    #[arg(
        short,
        long,
        value_name = "DISABLE_MLS",
        required = false,
        default_value_t = false
    )]
    disable_mls: bool,

    /// Do not run SLIM node in background.
    #[arg(
        short,
        long,
        value_name = "WITHOUT_SLIM",
        required = false,
        default_value_t = false
    )]
    without_slim: bool,

    /// Apps to run.
    #[arg(
        short,
        long,
        value_name = "APPS",
        required = false,
        default_value_t = 3
    )]
    apps: u32,

    /// connect to multiple remotes.
    #[arg(
        short,
        long,
        value_name = "MULTIPLE_REMOTES",
        required = false,
        default_value_t = false
    )]
    multiple_remotes: bool,
}

impl Args {
    pub fn disable_mls(&self) -> &bool {
        &self.disable_mls
    }

    pub fn without_slim(&self) -> &bool {
        &self.without_slim
    }

    pub fn apps(&self) -> &u32 {
        &self.apps
    }

    pub fn multiple_remotes(&self) -> &bool {
        &self.multiple_remotes
    }
}

async fn run_client_task(name: Name, moderator_name: Name, port: u16) -> Result<(), ServiceError> {
    /* this is the same */
    println!("client {} task starting...", name);

    let svc = build_client_service(port, DEFAULT_SERVICE_ID);

    let (_app, mut rx, conn_id, _svc) = create_and_subscribe_app(svc, &name).await?;

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
                                let session_arc = session_ctx.session_arc().unwrap();
                                let list = session_arc.participants_list().await?;
                                println!("Participant {}: session participants: {:?}", name_clone_session, list);
                                // check participants list
                                assert_eq!(list.len(), 2, "Expected 2 participants in the session");
                                assert!(list.iter().any(|n| n.components_strings() == name_clone_session.components_strings()),
                                    "Participants list should contain the client name");
                                assert!(list.iter().any(|n| n.components_strings() == moderator_name.components_strings()),
                                    "Participants list should contain the moderator name");

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
    let mls_enabled = !*args.disable_mls();
    let without_slim = *args.without_slim();
    let apps = *args.apps();
    let multiple_remotes = *args.multiple_remotes();

    println!(
        "run test with MLS = {} number of apps = {}, SLIM on = {}, multiple remotes = {}",
        mls_enabled, apps, !without_slim, multiple_remotes
    );

    // moderator name
    let moderator_name = Name::from_strings(["org", "ns", "moderator"]).with_id(1);

    let dataplane_port = reserve_local_port();

    // start slim node
    if !without_slim {
        tokio::spawn(async move {
            let _ = run_slim_node(dataplane_port).await;
        });
    }

    // start clients
    let mut tot_clients = apps;
    if !multiple_remotes {
        // if create a single replica of each client
        tot_clients = 1;
    }
    let mut clients = vec![];
    let client_1_name = Name::from_strings(["org", "ns", "client-1"]);
    let client_2_name = Name::from_strings(["org", "ns", "client-2"]);
    clients.push(client_1_name.clone());
    clients.push(client_2_name.clone());

    // create client-1 replicas
    for i in 0..tot_clients {
        let c = clients[0].clone().with_id(i.into());
        clients.push(c.clone());
        let moderator = moderator_name.clone();
        let port = dataplane_port;
        tokio::spawn(async move {
            let _ = run_client_task(c, moderator, port).await;
        });
    }

    // create client-2 replicas
    for i in 0..tot_clients {
        let c = clients[1].clone().with_id(i.into());
        clients.push(c.clone());
        let moderator = moderator_name.clone();
        let port = dataplane_port;
        tokio::spawn(async move {
            let _ = run_client_task(c, moderator, port).await;
        });
    }

    // start moderator

    let svc = build_client_service(dataplane_port, DEFAULT_SERVICE_ID);

    let (app, _rx, conn_id, _svc) = create_and_subscribe_app(svc, &moderator_name.clone()).await?;

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

    // wait for the creation of all the clients
    tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;

    let mut sessions = vec![];
    if multiple_remotes {
        // connect to two different clients using the discovery process
        // in the session layer
        for name in clients.iter().take(2) {
            let (session_ctx, completion_handle) = app
                .create_session(conf.clone(), name.clone(), None)
                .await
                .expect("error creating session");

            // Wait for session to be established
            completion_handle.await.expect("error establishing session");
            sessions.push(session_ctx);
        }
    } else {
        // connect to the same client using multiple sessions
        for _ in 0..2 {
            let (session_ctx, completion_handle) = app
                .create_session(conf.clone(), client_1_name.clone(), None)
                .await
                .expect("error creating session");

            // Wait for session to be established
            completion_handle.await.expect("error establishing session");
            sessions.push(session_ctx);
        }
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;

    for session in &sessions {
        let session_arc = session.session_arc().unwrap();
        let list = session_arc.participants_list().await?;
        println!("Moderator: session participants: {:?}", list);
        if multiple_remotes {
            // check that each session has 2 participants: moderator and one of the two clients
            assert_eq!(
                list.len(),
                2,
                "Expected 2 participants in the moderator session"
            );
            assert!(
                list.iter()
                    .any(|n| n.components_strings() == moderator_name.components_strings()),
                "Participants list should contain the moderator name"
            );
            assert!(
                list.iter().any(
                    |n| n.components_strings() == client_1_name.components_strings()
                        || n.components_strings() == client_2_name.components_strings()
                ),
                "Participants list should contain the client name"
            );
        } else {
            // check that each session has 2 participants: moderator and client-1
            assert_eq!(
                list.len(),
                2,
                "Expected 2 participants in the moderator session"
            );
            assert!(
                list.iter()
                    .any(|n| n.components_strings() == moderator_name.components_strings()),
                "Participants list should contain the moderator name"
            );
            assert!(
                list.iter()
                    .any(|n| n.components_strings() == client_1_name.components_strings()),
                "Participants list should contain the client-1 name"
            );
        }
    }

    // listen for messages
    let max_packets = 50;
    // for each receiver store received messages count
    let recv_msgs = Arc::new(RwLock::new(HashMap::new()));
    let recv_msgs_clone = recv_msgs.clone();

    // Create a channel to signal when all messages are received from all sessions
    let (all_received_tx, all_received_rx) = tokio::sync::oneshot::channel();
    let all_received_tx = Arc::new(parking_lot::Mutex::new(Some(all_received_tx)));

    // Calculate expected total messages
    // 2*max_packets (initial) + max_packets (after closing one session) = 3*max_packets
    let expected_total = 3 * max_packets;

    // Expected per-session packets: 2*max_packets for session 0, 1*max_packets for session 1
    let expected_packets_session0 = 2 * max_packets;
    let expected_packets_session1 = max_packets;

    // Clone the Arc to session for later use
    let mut sessions_arc = vec![];
    for (idx, session_ctx) in sessions.into_iter().enumerate() {
        let session_arc = session_ctx.session_arc().unwrap();
        sessions_arc.push(session_arc);

        let recv_msgs_clone = recv_msgs_clone.clone();
        let all_received_tx_clone = all_received_tx.clone();

        // Choose expected packets for this session
        let expected_packets = if idx == 0 {
            expected_packets_session0
        } else {
            expected_packets_session1
        };

        session_ctx.spawn_receiver(move |mut rx, _weak| async move {
            let mut received_packets: u32 = 0;
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
                                let p =
                                    &publish.get_payload().as_application_payload().unwrap().blob;
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

                                // Check if we've received all expected messages on this session
                                received_packets += 1;
                                if received_packets == expected_packets {
                                    let total: u32 = recv_msgs_clone.read().values().sum();
                                    if total >= expected_total {
                                        println!(
                                            "Received all {} messages, signaling completion",
                                            expected_total
                                        );
                                        if let Some(tx) = all_received_tx_clone.lock().take() {
                                            let _ = tx.send(());
                                        }
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
    }

    let msg_payload_str = "hello there";
    let p = msg_payload_str.as_bytes().to_vec();
    let mut completion_handlers = vec![];
    for i in 0..max_packets {
        println!("main: send message {}", i);

        for (idx, session_arc) in sessions_arc.iter().enumerate() {
            let client_name = if idx == 0 {
                client_1_name.clone()
            } else {
                client_2_name.clone()
            };

            let completion_handler = session_arc
                .publish(&client_name, p.clone(), None, None)
                .await
                .expect("an error occurred sending publication from moderator");

            completion_handlers.push(completion_handler);
        }
    }

    // wait for all messages to be sent
    futures::future::try_join_all(completion_handlers)
        .await
        .expect("an error occurred waiting for publication completion from moderator");

    tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;

    // close the sessions[1] and send another max_packets messages to
    // the remaining session
    let handle = app
        .delete_session(sessions_arc[1].as_ref())
        .expect("an error occurred deleting the session");

    // await handle
    handle
        .await
        .expect("an error occurred waiting for session deletion");

    let mut completion_handlers = vec![];
    for i in max_packets..max_packets * 2 {
        println!("main: send message {}", i);

        let completion_handler = sessions_arc[0]
            .publish(&client_1_name, p.clone(), None, None)
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

    // delete the remaining sessions
    let handle = app
        .delete_session(sessions_arc[0].as_ref())
        .expect("an error occurred deleting the session");

    // await handle
    handle
        .await
        .expect("an error occurred waiting for session deletion");

    // the total number of packets received must be max_packets * sessions.len()
    let mut sum = 0;
    let mut found_sender = HashSet::new();
    for (c, n) in recv_msgs.read().iter() {
        sum += *n;
        if *n != 0 {
            found_sender.insert(c.clone());
        }
        println!("received {} messages from {}", n, c);
    }

    // if multiple-remotes is set we expect messages from exactly 2 clients,
    // otherwise all messages must come from client_1_name.with_id(0)
    if multiple_remotes && found_sender.len() != 2 {
        println!(
            "expected messages from 2 clients, but got messages from {} clients. test failed",
            found_sender.len(),
        );
        std::process::exit(1);
    } else if !multiple_remotes
        && (found_sender.len() != 1 || !found_sender.contains(&client_1_name.clone().with_id(0)))
    {
        println!(
            "expected messages only from {}, but got messages from {:?}. test failed",
            client_1_name.clone().with_id(0),
            found_sender,
        );
        std::process::exit(1);
    }

    if sum != expected_total {
        println!(
            "expected {} packets, received {}. test failed",
            expected_total, sum
        );
        std::process::exit(1);
    }

    println!("test succeeded");
    Ok(())
}
