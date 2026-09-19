// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use slim_datapath::api::{ProtoName, ProtoSessionType};
use slim_session::{Notification, SessionConfig, session_config::MlsSettings};
use slim_testing::{
    binaries::require_slim_binary,
    build_client_service,
    common::create_and_subscribe_app,
    helpers::{
        ProcessLogWatcher, new_temp_dir, reserve_port, spawn_slim, terminate_session,
        write_temp_config,
    },
};

fn testdata_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn server_replacements(port: u16) -> HashMap<String, String> {
    HashMap::from([("0.0.0.0:46357".to_string(), format!("0.0.0.0:{port}"))])
}

#[tokio::test]
async fn invite_participant_without_set_route_uses_default_gateway() {
    let temp_dir = new_temp_dir("slim-default-gateway-");
    let server_port = reserve_port();

    let server_config = write_temp_config(
        temp_dir.path(),
        &testdata_dir().join("server.yaml"),
        "server.yaml",
        &server_replacements(server_port),
    );

    let slim = require_slim_binary();

    let mut slim_process = Some(spawn_slim(&slim, &server_config));
    let slim_logs = ProcessLogWatcher::attach(slim_process.as_mut().expect("SLIM process missing"));

    slim_logs
        .wait_contains("dataplane server started", Duration::from_secs(15))
        .unwrap_or_else(|output| {
            terminate_session(&mut slim_process, Duration::from_secs(5));
            panic!("SLIM server did not start:\n{output}");
        });

    // Create moderator service with one Edge connection.
    let moderator_name = ProtoName::from_strings(["org", "ns", "moderator"]);
    let participant_name = ProtoName::from_strings(["org", "ns", "participant"]);

    let channel_name = ProtoName::from_strings(["org", "ns", "channel"]);

    let moderator_service = build_client_service(server_port, &moderator_name);
    let participant_service = build_client_service(server_port, &participant_name);

    let (moderator_app, _moderator_rx, _moderator_conn, _moderator_service) =
        create_and_subscribe_app(moderator_service, &moderator_name)
            .await
            .expect("failed to create and subscribe moderator app");
    let (participant_app, mut participant_rx, _participant_conn, _participant_service) =
        create_and_subscribe_app(participant_service, &participant_name)
            .await
            .expect("failed to create and subscribe participant app");

    let participant_listener = tokio::spawn(async move {
        loop {
            match participant_rx.recv().await {
                Some(Ok(Notification::NewSession(session_ctx))) => {
                    session_ctx.spawn_receiver(|mut rx, _weak| async move {
                        while rx.recv().await.is_some() {}
                    });
                    return;
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    panic!("participant received error: {e}");
                }
                None => {
                    panic!("participant channel closed");
                }
            }
        }
    });

    let session_config = SessionConfig {
        session_type: ProtoSessionType::Multicast,
        max_retries: Some(10),
        interval: Some(Duration::from_secs(1)),
        mls_settings: Some(MlsSettings::default()),
        initiator: true,
        metadata: HashMap::new(),
    };

    // Create participant service with one Edge connection.
    let (session_context, session_completion) = moderator_app
        .create_session(session_config, channel_name, None)
        .await
        .expect("failed to create multicast session");

    session_completion
        .await
        .expect("multicast session creation failed");

    let session = session_context
        .session_arc()
        .expect("moderator session was dropped");

    // Important: do not call set_route().
    // Call invite_participant(participant_name).

    let invite_completion = session
        .invite_participant(&participant_name)
        .await
        .expect("failed to invite participant");

    let _ = tokio::time::timeout(Duration::from_secs(15), invite_completion)
        .await
        .expect("invite completion failed");

    // Assert that the participant receives Notification::NewSession.
    let _ = tokio::time::timeout(Duration::from_secs(15), participant_listener)
        .await
        .expect("participant listener timed out");

    terminate_session(&mut slim_process, Duration::from_secs(5));

    drop(participant_app);
    drop(moderator_app);
}
