// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::time::Duration;

use clap::Parser;
use slim_datapath::api::ProtoName as Name;
use slim_service::{ServiceError, SlimHeaderFlags};
use slim_session::{Notification, SessionConfig};
use slim_testing::build_client_service;
use slim_testing::common::{create_and_subscribe_app, reserve_local_port, run_slim_node};
use slim_tracing::TracingConfiguration;
use tracing::{Instrument, error, info, info_span};

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

async fn run_participant_task(name: Name, port: u16) -> Result<(), ServiceError> {
    info!("task starting on port {}", port);

    let svc = build_client_service(port, &name);
    let (_app, mut rx, _conn_id, _svc) = create_and_subscribe_app(svc, &name).await?;

    let moderator = Name::from_strings(["org", "ns", "moderator"]);
    let channel_name = Name::from_strings(["channel", "channel", "channel"]);

    let name_clone = name.clone();
    let moderator_clone = moderator.clone();
    let channel_name_clone = channel_name.clone();
    loop {
        tokio::select! {
            msg_result = rx.recv() => {
                match msg_result {
                    None => { info!("end of stream, close app"); break; }
                    Some(res) => match res {
                        Ok(notification) => match notification {
                            Notification::NewSession(session_ctx) => {
                                info!("new session created");
                                let session_moderator_clone = moderator_clone.clone();
                                let session_channel_name_clone = channel_name_clone.clone();
                                let session_name = name_clone.clone();
                                session_ctx.spawn_receiver(move |mut rx, weak| {
                                    async move {
                                        loop {
                                            match rx.recv().await {
                                                None => {
                                                    info!("session receiver: end of stream");
                                                    break;
                                                }
                                                Some(Ok(msg)) => {
                                                    if let Some(slim_datapath::api::ProtoPublishType(publish)) = msg.message_type.as_ref() {
                                                        let publisher = msg.get_slim_header().get_source();
                                                        let msg_id = msg.get_id();
                                                        let blob = &publish.get_payload().as_application_payload().unwrap().blob;
                                                        if let Ok(val) = String::from_utf8(blob.to_vec()) {
                                                            if publisher.str_components() == session_moderator_clone.str_components() {
                                                                if val != *"hello there" { continue; }
                                                                info!(msg_id, "received message from moderator, sending ack");
                                                                let payload = msg_id.to_ne_bytes().to_vec();
                                                                let flags = SlimHeaderFlags::new(10, None, None, None, None);
                                                                if let Some(session_arc) = weak.upgrade() &&
                                                                    session_arc.publish_with_flags(&session_channel_name_clone, flags, payload, None, None).await.is_err() {
                                                                    panic!("an error occurred sending publication from moderator");
                                                                }
                                                            }
                                                        } else { error!("error parsing message blob"); }
                                                    }
                                                }
                                                Some(Err(e)) => {
                                                    error!("session receiver error: {:?}", e);
                                                    break;
                                                }
                                            }
                                        }
                                    }.instrument(info_span!("session_receiver", participant = %session_name))
                                });
                            }
                            _ => {
                                info!("unexpected notification type");
                                continue;
                            }
                        }
                        Err(e) => { error!("received error message: {:?}", e); }
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

    let tracing = TracingConfiguration::default();
    let _guard = tracing.setup_tracing_subscriber();

    if mls_enabled {
        info!("start test with mls enabled");
    } else {
        info!("start test with mls disabled");
    }
    let dataplane_port = reserve_local_port();
    info!(dataplane_port, "reserved data plane port");

    // start slim node
    tokio::spawn(
        async move {
            let _ = run_slim_node(dataplane_port).await;
        }
        .instrument(info_span!("slim_node", port = dataplane_port)),
    );

    // start clients
    let tot_participants = 5;
    let mut participants = vec![];

    for i in 0..tot_participants {
        let p = Name::from_strings(["org", "ns", &format!("t{}", i)]);
        participants.push(p.clone());
        let port = dataplane_port;
        let span = info_span!("participant", name = %p);
        tokio::spawn(
            async move {
                let _ = run_participant_task(p, port).await;
            }
            .instrument(span),
        );
    }

    // wait for all the processes to start
    tokio::time::sleep(tokio::time::Duration::from_millis(10000)).await;

    // start moderator
    let moderator_span = info_span!("moderator");
    let _moderator_guard = moderator_span.enter();

    let name = Name::from_strings(["org", "ns", "moderator"]);
    let channel_name = Name::from_strings(["channel", "channel", "channel"]);

    let svc = build_client_service(dataplane_port, &name);

    let (app, _rx, conn_id, _svc) = create_and_subscribe_app(svc, &name).await?;

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
        info!("inviting participant {}", c);
        let handler = session_ctx
            .session_arc()
            .unwrap()
            .invite_participant(c)
            .await
            .expect("error sending invite message");
        handler
            .await
            .expect("error awaiting the execution of the participant invite");
    }

    // listen for messages
    let max_packets = 100;
    let recv_msgs = std::sync::Arc::new(parking_lot::RwLock::new(vec![0; max_packets]));
    let recv_msgs_clone = recv_msgs.clone();

    // Clone the Arc to session for later use
    let session_arc = session_ctx.session_arc().unwrap();

    let list = session_arc.participants_list().await?;
    println!("Moderator: session participants: {:?}", list);
    assert_eq!(
        list.len(),
        tot_participants, // moderator + N-1 participants
        "Expected {} participants in the moderator session",
        tot_participants
    );

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
        println!("Moderator: send message {}", i);

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

            let handler = session_arc
                .remove_participant(&participants[to_remove])
                .await
                .expect("error removing participant");
            handler
                .await
                .expect("error awaiting the execution of the participant remove");

            let list = session_arc.participants_list().await?;
            println!("Moderator: session participants after remove: {:?}", list);
            assert_eq!(
                list.len(),
                tot_participants - 1, // moderator + N-2 participants
                "Expected {} participants in the moderator session",
                tot_participants - 1
            );
            assert!(
                list.iter()
                    .all(|n| n.str_components() != participants[to_remove].str_components()),
                "Participants to remove is still present in the session"
            );

            let handler = session_arc
                .invite_participant(&participants[to_add])
                .await
                .expect("error adding participant");
            handler
                .await
                .expect("error awaiting the execution of the participant add");

            let list = session_arc.participants_list().await?;
            println!("Moderator: session participants after remove: {:?}", list);
            assert_eq!(
                list.len(),
                tot_participants, // moderator + N-1 participants
                "Expected {} participants in the moderator session",
                tot_participants
            );
            assert!(
                list.iter()
                    .any(|n| n.str_components() == participants[to_add].str_components()),
                "Participants to add is not present in the session"
            );

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

    // delete session
    let handle = app.delete_session(session_arc.as_ref())?;
    drop(session_arc);
    handle.await.expect("error deleting session");
    info!("test succeeded");
    Ok(())
}
