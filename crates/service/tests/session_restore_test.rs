// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Integration test for restoring a persisted multicast MLS session over a real
//! datapath.
//!
//! Scenario: a moderator and a participant form an MLS-encrypted multicast
//! group and exchange an encrypted message. The participant is then "restarted"
//! — its app/service are dropped and a brand-new app is created with the *same
//! identity* and the *same persistence directory*. Calling `restore_sessions()`
//! must bring the session back (reloading the MLS group and re-subscribing to
//! the channel) without any re-invite, and the restored participant must be
//! able to decrypt a fresh message published by the moderator.
//!
//! Note: the group membership is intentionally left unchanged while the
//! participant is down (the moderator does not remove it within the short
//! restart window), which is the precondition under which restore is valid —
//! see the epoch-drift caveat on the persistence design.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::api::ProtoName as Name;
use slim_persistence::PersistenceConfig;
use slim_service::app::App;
use slim_service::{Service, SlimHeaderFlags};
use slim_session::session_config::MlsSettings;
use slim_session::{Direction, Notification, SessionConfig};
use slim_testing::build_client_service;
use slim_testing::common::{reserve_local_port, run_slim_node};
use slim_testing::utils::TEST_VALID_SECRET;

type TestApp = App<SharedSecret, SharedSecret>;

/// Create a client app connected to the node on `port`, backed by an encrypted
/// persistence store under `dir`, and subscribed to its own name.
async fn create_persistent_app(
    port: u16,
    name: &Name,
    secret: SharedSecret,
    dir: &Path,
) -> (
    TestApp,
    tokio::sync::mpsc::Receiver<Result<Notification, slim_session::SessionError>>,
    u64,
    Service,
) {
    let svc = build_client_service(port, name);
    // The persistence store is keyed by the app name, and the app's identity
    // (id + secret + MLS keypair) is saved into it on first run and restored on
    // subsequent runs. So a restart can hand in a brand-new `SharedSecret` with
    // the same name + secret; the persisted identity is adopted from disk.
    let (app, rx) = svc
        .create_app_with_direction_and_persistence(
            name,
            secret.clone(),
            secret,
            Direction::Bidirectional,
            Some(PersistenceConfig::new(dir)),
        )
        .expect("failed to create persistent app");

    svc.run().await.expect("failed to run service");

    let conn_id = svc
        .get_connection_id(&svc.config().dataplane_clients()[0].endpoint)
        .expect("no connection id");

    app.subscribe(name, Some(conn_id))
        .await
        .expect("failed to subscribe app to its own name");

    (app, rx, conn_id, svc)
}

/// Drive a session receiver, pushing every decrypted string payload into `sink`.
fn collect_messages(ctx: slim_session::context::SessionContext, sink: Arc<Mutex<Vec<String>>>) {
    ctx.spawn_receiver(move |mut rx, _weak| async move {
        while let Some(Ok(msg)) = rx.recv().await {
            if let Some(slim_datapath::api::ProtoPublishType(publish)) = msg.message_type.as_ref()
                && let Ok(payload) = publish.get_payload().as_application_payload()
                && let Ok(text) = String::from_utf8(payload.blob.to_vec())
            {
                sink.lock().push(text);
            }
        }
    });
}

/// Wait until `sink` contains `expected`, or fail after `timeout`.
async fn wait_for_message(sink: &Arc<Mutex<Vec<String>>>, expected: &str, timeout: Duration) {
    let poll = async {
        while !sink.lock().iter().any(|m| m == expected) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    if tokio::time::timeout(timeout, poll).await.is_err() {
        panic!("timed out waiting for {expected:?}; got {:?}", sink.lock());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multicast_mls_session_restore() {
    // 1. Start the SLIM node.
    let port = reserve_local_port();
    tokio::spawn(async move {
        let _ = run_slim_node(port).await;
    });
    // Give the node a moment to accept connections.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let moderator_name = Name::from_strings(["org", "ns", "moderator"]);
    let participant_name = Name::from_strings(["org", "ns", "participant"]);
    let channel = Name::from_strings(["channel", "restore", "test"]);

    let moderator_dir = tempfile::tempdir().unwrap();
    let participant_dir = tempfile::tempdir().unwrap();

    // Identities are created once; the participant's is cloned across the
    // restart so its identity (and thus its persistence keys) stay stable.
    let moderator_secret = SharedSecret::new("moderator-identity", TEST_VALID_SECRET).unwrap();
    let participant_secret = SharedSecret::new("participant-identity", TEST_VALID_SECRET).unwrap();

    // 2. Moderator + participant apps, both persistent.
    let (moderator_app, _moderator_rx, moderator_conn, _moderator_svc) = create_persistent_app(
        port,
        &moderator_name,
        moderator_secret,
        moderator_dir.path(),
    )
    .await;
    let (participant_app, mut participant_rx, _participant_conn, participant_svc) =
        create_persistent_app(
            port,
            &participant_name,
            participant_secret.clone(),
            participant_dir.path(),
        )
        .await;

    // 3. Participant: on NewSession, collect decrypted messages.
    let pre_restart = Arc::new(Mutex::new(Vec::<String>::new()));
    let pre_restart_sink = pre_restart.clone();
    let participant_listener = tokio::spawn(async move {
        while let Some(Ok(Notification::NewSession(ctx))) = participant_rx.recv().await {
            collect_messages(ctx, pre_restart_sink.clone());
        }
    });

    // 4. Moderator creates a multicast MLS session and invites the participant.
    let config = SessionConfig {
        session_type: slim_datapath::api::ProtoSessionType::Multicast,
        max_retries: Some(10),
        interval: Some(Duration::from_secs(1)),
        mls_settings: Some(MlsSettings::default()),
        initiator: true,
        metadata: Default::default(),
    };
    let (session_ctx, completion) = moderator_app
        .create_session(config, channel.clone(), None)
        .await
        .expect("failed to create session");
    completion.await.expect("session establishment failed");

    moderator_app
        .set_route(&participant_name, moderator_conn)
        .await
        .expect("failed to set route to participant");

    let session = session_ctx.session_arc().expect("no session arc");
    session
        .invite_participant(&participant_name)
        .await
        .expect("invite failed")
        .await
        .expect("invite completion failed");

    // 5. Moderator publishes an encrypted message; participant must decrypt it.
    session
        .publish_with_flags(
            &channel,
            SlimHeaderFlags::new(10, None, None, None, None),
            b"hello-before-restart".to_vec(),
            None,
            None,
        )
        .await
        .expect("publish #1 failed");
    wait_for_message(
        &pre_restart,
        "hello-before-restart",
        Duration::from_secs(10),
    )
    .await;

    // 6. "Restart" the participant: drop its app + service + listener.
    drop(participant_app);
    drop(participant_svc);
    participant_listener.abort();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 7. True restart: a brand-new identity object (same name + secret, but a
    // fresh random id and MLS keypair — as after a real process restart),
    // pointed at the same persistence dir. The store is keyed by the app name,
    // so the persisted app identity is loaded from disk and adopted, and restore
    // re-establishes routing over the live upstream connection (`conn2`) — no
    // reliance on the pre-restart identity object surviving in memory.
    let restarted_secret = SharedSecret::new("participant-identity", TEST_VALID_SECRET).unwrap();
    let (participant_app2, _participant_rx2, conn2, _participant_svc2) = create_persistent_app(
        port,
        &participant_name,
        restarted_secret,
        participant_dir.path(),
    )
    .await;

    let restored = participant_app2
        .restore_sessions(conn2)
        .await
        .expect("restore_sessions failed");
    assert_eq!(restored.len(), 1, "exactly one session should be restored");

    let post_restart = Arc::new(Mutex::new(Vec::<String>::new()));
    collect_messages(restored.into_iter().next().unwrap(), post_restart.clone());

    // Give the re-subscription time to propagate to the node.
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // 8. Moderator publishes again; the restored participant must decrypt it
    //    using the reloaded MLS group — no re-invite occurred.
    session
        .publish_with_flags(
            &channel,
            SlimHeaderFlags::new(10, None, None, None, None),
            b"hello-after-restart".to_vec(),
            None,
            None,
        )
        .await
        .expect("publish #2 failed");
    wait_for_message(
        &post_restart,
        "hello-after-restart",
        Duration::from_secs(10),
    )
    .await;
}
