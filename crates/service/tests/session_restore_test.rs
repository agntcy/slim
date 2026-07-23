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

/// Create a non-persistent client app on `port`, subscribed to its own name.
async fn create_app(
    port: u16,
    name: &Name,
    secret: SharedSecret,
) -> (
    TestApp,
    tokio::sync::mpsc::Receiver<Result<Notification, slim_session::SessionError>>,
    u64,
    Service,
) {
    let svc = build_client_service(port, name);
    let (app, rx) = svc
        .create_app(name, secret.clone(), secret)
        .expect("failed to create app");
    svc.run().await.expect("failed to run service");
    let conn_id = svc
        .get_connection_id(&svc.config().dataplane_clients()[0].endpoint)
        .expect("no connection id");
    app.subscribe(name, Some(conn_id))
        .await
        .expect("failed to subscribe");
    (app, rx, conn_id, svc)
}

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

/// Wait until `sink` contains any message matching `pred`, or fail after `timeout`.
async fn wait_for_any_message<F>(sink: &Arc<Mutex<Vec<String>>>, pred: F, timeout: Duration)
where
    F: Fn(&str) -> bool,
{
    let poll = async {
        while !sink.lock().iter().any(|m| pred(m.as_str())) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    if tokio::time::timeout(timeout, poll).await.is_err() {
        panic!(
            "timed out waiting for a matching message; got {:?}",
            sink.lock()
        );
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

/// End-to-end: restart the **moderator** and have it publish again.
///
/// Exercises two things a restored *sender* needs:
/// 1. Repopulating the session sender's endpoint list (`restore_reconnect` now
///    calls `add_endpoint`), so the restored publish actually reaches the peer.
/// 2. Resuming the outbound data-message sequence — the sender persists
///    `next_id` per publish and reloads it on restore, so post-restart messages
///    carry ids *after* the ones the still-live participant already saw. Without
///    this the participant dedups them ("possibly DUP … drop it") and the fresh
///    message never surfaces.
#[tokio::test(flavor = "multi_thread")]
async fn test_multicast_mls_moderator_restart_republish() {
    // 1. Start the SLIM node.
    let port = reserve_local_port();
    tokio::spawn(async move {
        let _ = run_slim_node(port).await;
    });
    tokio::time::sleep(Duration::from_millis(500)).await;

    let moderator_name = Name::from_strings(["org", "ns", "moderator"]);
    let participant_name = Name::from_strings(["org", "ns", "participant"]);
    let channel = Name::from_strings(["channel", "restart", "test"]);

    let moderator_dir = tempfile::tempdir().unwrap();
    let participant_dir = tempfile::tempdir().unwrap();

    let moderator_secret = SharedSecret::new("moderator-identity", TEST_VALID_SECRET).unwrap();
    let participant_secret = SharedSecret::new("participant-identity", TEST_VALID_SECRET).unwrap();

    // 2. Participant app — stays alive for the whole test, collecting every
    //    decrypted message across its session's lifetime.
    let (_participant_app, mut participant_rx, _participant_conn, _participant_svc) =
        create_persistent_app(
            port,
            &participant_name,
            participant_secret,
            participant_dir.path(),
        )
        .await;

    let received = Arc::new(Mutex::new(Vec::<String>::new()));
    let received_sink = received.clone();
    let _participant_listener = tokio::spawn(async move {
        while let Some(Ok(Notification::NewSession(ctx))) = participant_rx.recv().await {
            collect_messages(ctx, received_sink.clone());
        }
    });

    // 3. Moderator (first run): create the multicast MLS session + invite.
    let (moderator_app, _moderator_rx, moderator_conn, moderator_svc) = create_persistent_app(
        port,
        &moderator_name,
        moderator_secret,
        moderator_dir.path(),
    )
    .await;

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

    // 4. Baseline: moderator publishes, participant receives.
    session
        .publish_with_flags(
            &channel,
            SlimHeaderFlags::new(10, None, None, None, None),
            b"before-moderator-restart".to_vec(),
            None,
            None,
        )
        .await
        .expect("publish #1 failed");
    wait_for_message(
        &received,
        "before-moderator-restart",
        Duration::from_secs(10),
    )
    .await;

    // 5. Restart the moderator: drop its session, app and service.
    drop(session);
    drop(session_ctx);
    drop(moderator_app);
    drop(moderator_svc);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 6. Recreate the moderator (same name + secret + persistence dir, fresh id
    //    and MLS keypair as after a real restart) and restore its session.
    let restarted_secret = SharedSecret::new("moderator-identity", TEST_VALID_SECRET).unwrap();
    let (moderator_app2, _moderator_rx2, conn2, _moderator_svc2) = create_persistent_app(
        port,
        &moderator_name,
        restarted_secret,
        moderator_dir.path(),
    )
    .await;

    // Re-establish the app-level route to the participant (as an app does on
    // startup) before restoring.
    moderator_app2
        .set_route(&participant_name, conn2)
        .await
        .expect("failed to set route to participant after restart");

    let restored = moderator_app2
        .restore_sessions(conn2)
        .await
        .expect("restore_sessions failed");
    assert_eq!(
        restored.len(),
        1,
        "the moderator session should be restored"
    );

    // Give the re-subscription/routing time to propagate to the node.
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // 7. The restored moderator publishes again; the still-live participant must
    //    receive it — i.e. the restored session actually fans out to its member.
    let restored_session = restored
        .into_iter()
        .next()
        .unwrap()
        .session_arc()
        .expect("no session arc on restored moderator session");
    restored_session
        .publish_with_flags(
            &channel,
            SlimHeaderFlags::new(10, None, None, None, None),
            b"after-moderator-restart".to_vec(),
            None,
            None,
        )
        .await
        .expect("publish #2 (after restart) failed");
    wait_for_message(
        &received,
        "after-moderator-restart",
        Duration::from_secs(10),
    )
    .await;
}

/// Moderator invites two members, member2 goes offline (close), moderator then
/// invites a third member (advancing the MLS epoch), and member2 rejoins.
/// Because the epoch has moved on, the initial rejoin is NACKed and member2
/// automatically sends a RejoinRequest; after the moderator processes it and
/// sends a welcome, member2 should be able to decrypt fresh publishes.
#[tokio::test(flavor = "multi_thread")]
async fn test_participant_rejoin_after_epoch_advance() {
    // 1. Start SLIM node.
    let port = reserve_local_port();
    tokio::spawn(async move {
        let _ = run_slim_node(port).await;
    });
    tokio::time::sleep(Duration::from_millis(500)).await;

    let moderator_name = Name::from_strings(["org", "ns", "moderator2"]);
    let member1_name = Name::from_strings(["org", "ns", "member1"]);
    let member2_name = Name::from_strings(["org", "ns", "member2"]);
    let member3_name = Name::from_strings(["org", "ns", "member3"]);
    let channel = Name::from_strings(["channel", "rejoin", "epoch"]);

    // 2. Create all four apps (no persistence needed — close/rejoin is in-memory).
    let (moderator_app, _mod_rx, mod_conn, _mod_svc) = create_app(
        port,
        &moderator_name,
        SharedSecret::new("mod2", TEST_VALID_SECRET).unwrap(),
    )
    .await;
    let (_member1_app, mut member1_rx, _m1_conn, _m1_svc) = create_app(
        port,
        &member1_name,
        SharedSecret::new("member1", TEST_VALID_SECRET).unwrap(),
    )
    .await;
    let (_member2_app, mut member2_rx, _m2_conn, _m2_svc) = create_app(
        port,
        &member2_name,
        SharedSecret::new("member2", TEST_VALID_SECRET).unwrap(),
    )
    .await;
    let (_member3_app, mut member3_rx, _m3_conn, _m3_svc) = create_app(
        port,
        &member3_name,
        SharedSecret::new("member3", TEST_VALID_SECRET).unwrap(),
    )
    .await;

    // 3. Set routes from the moderator to every member.
    for name in [&member1_name, &member2_name, &member3_name] {
        moderator_app
            .set_route(name, mod_conn)
            .await
            .expect("set_route failed");
    }

    // 4. Create the multicast MLS session.
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
        .expect("create_session failed");
    completion.await.expect("session establishment failed");
    let session = session_ctx.session_arc().expect("no session arc");

    // 5. Invite member1 and member2.
    let member1_joined = Arc::new(Mutex::new(Vec::<String>::new()));
    let member2_messages = Arc::new(Mutex::new(Vec::<String>::new()));

    // member1 listener: just collect messages.
    let m1_sink = member1_joined.clone();
    tokio::spawn(async move {
        while let Some(Ok(Notification::NewSession(ctx))) = member1_rx.recv().await {
            collect_messages(ctx, m1_sink.clone());
        }
    });

    // member2 listener: save the session arc before spawning the collector so we
    // can call close()/rejoin() on it later.
    let m2_sink = member2_messages.clone();
    let (member2_arc_tx, member2_arc_rx) = tokio::sync::oneshot::channel();
    let mut member2_arc_tx = Some(member2_arc_tx);
    tokio::spawn(async move {
        while let Some(Ok(Notification::NewSession(ctx))) = member2_rx.recv().await {
            // Send the session arc to the test body before spawning the receiver.
            if let (Some(arc), Some(tx)) = (ctx.session_arc(), member2_arc_tx.take()) {
                let _ = tx.send(arc);
            }
            collect_messages(ctx, m2_sink.clone());
        }
    });

    // member3 listener: only needed so the invite completes; discard messages.
    tokio::spawn(async move {
        while let Some(Ok(Notification::NewSession(_ctx))) = member3_rx.recv().await {}
    });

    session
        .invite_participant(&member1_name)
        .await
        .expect("invite member1 failed")
        .await
        .expect("invite member1 completion failed");

    session
        .invite_participant(&member2_name)
        .await
        .expect("invite member2 failed")
        .await
        .expect("invite member2 completion failed");

    // Retrieve member2's session arc (sent by the listener above).
    let member2_session = tokio::time::timeout(Duration::from_secs(10), member2_arc_rx)
        .await
        .expect("timed out waiting for member2 session arc")
        .expect("member2 arc channel dropped");

    // 6. Baseline: moderator publishes; member2 must receive it.
    session
        .publish_with_flags(
            &channel,
            SlimHeaderFlags::new(10, None, None, None, None),
            b"before-close".to_vec(),
            None,
            None,
        )
        .await
        .expect("baseline publish failed");
    wait_for_message(&member2_messages, "before-close", Duration::from_secs(10)).await;

    // 7. Member2 goes offline.
    member2_session
        .close()
        .await
        .expect("close failed")
        .await
        .expect("close completion failed");

    // 8. Moderator invites member3, advancing the MLS epoch.
    session
        .invite_participant(&member3_name)
        .await
        .expect("invite member3 failed")
        .await
        .expect("invite member3 completion failed");

    // 9. Member2 rejoins. Because the epoch has advanced the first attempt is
    //    NACKed; the participant layer automatically sends a RejoinRequest and the
    //    completion handle resolves only after the welcome is processed.
    member2_session
        .rejoin()
        .await
        .expect("rejoin failed")
        .await
        .expect("rejoin completion failed");

    // Give re-subscription time to propagate.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 10. Moderator publishes again; member2 must decrypt it with the new epoch.
    session
        .publish_with_flags(
            &channel,
            SlimHeaderFlags::new(10, None, None, None, None),
            b"after-rejoin".to_vec(),
            None,
            None,
        )
        .await
        .expect("post-rejoin publish failed");
    wait_for_message(&member2_messages, "after-rejoin", Duration::from_secs(10)).await;
}

/// Same scenario as `test_participant_rejoin_after_epoch_advance` but member2
/// fully disconnects (app + service dropped) and is recreated from persistence.
///
/// After the restore, member2's MLS state has the old epoch (before member3 was
/// invited). The epoch-mismatch rejoin is triggered automatically by the next
/// heartbeat from the moderator (~10 s interval), so the test keeps publishing
/// every second and waits for the first message member2 can actually decrypt.
#[tokio::test(flavor = "multi_thread")]
async fn test_participant_rejoin_after_epoch_advance_with_persistence() {
    // 1. Start SLIM node.
    let port = reserve_local_port();
    tokio::spawn(async move {
        let _ = run_slim_node(port).await;
    });
    tokio::time::sleep(Duration::from_millis(500)).await;

    let moderator_name = Name::from_strings(["org", "ns", "moderator3"]);
    let member1_name = Name::from_strings(["org", "ns", "member1p"]);
    let member2_name = Name::from_strings(["org", "ns", "member2p"]);
    let member3_name = Name::from_strings(["org", "ns", "member3p"]);
    let channel = Name::from_strings(["channel", "rejoin", "persist"]);

    let member2_dir = tempfile::tempdir().unwrap();

    // 2. Create apps. Member2 is persistent so it can be restored later.
    let (moderator_app, _mod_rx, mod_conn, _mod_svc) = create_app(
        port,
        &moderator_name,
        SharedSecret::new("mod3", TEST_VALID_SECRET).unwrap(),
    )
    .await;
    let (_member1_app, mut member1_rx, _m1_conn, _m1_svc) = create_app(
        port,
        &member1_name,
        SharedSecret::new("member1p", TEST_VALID_SECRET).unwrap(),
    )
    .await;
    let member2_secret = SharedSecret::new("member2p", TEST_VALID_SECRET).unwrap();
    let (member2_app, mut member2_rx, _m2_conn, member2_svc) = create_persistent_app(
        port,
        &member2_name,
        member2_secret.clone(),
        member2_dir.path(),
    )
    .await;
    let (_member3_app, mut member3_rx, _m3_conn, _m3_svc) = create_app(
        port,
        &member3_name,
        SharedSecret::new("member3p", TEST_VALID_SECRET).unwrap(),
    )
    .await;

    // 3. Routes from moderator to all members.
    for name in [&member1_name, &member2_name, &member3_name] {
        moderator_app
            .set_route(name, mod_conn)
            .await
            .expect("set_route failed");
    }

    // 4. Create the multicast MLS session.
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
        .expect("create_session failed");
    completion.await.expect("session establishment failed");
    let session = session_ctx.session_arc().expect("no session arc");

    // 5. Set up listeners and invite member1 + member2.
    let member2_messages = Arc::new(Mutex::new(Vec::<String>::new()));

    tokio::spawn(async move {
        while let Some(Ok(Notification::NewSession(_ctx))) = member1_rx.recv().await {}
    });
    tokio::spawn(async move {
        while let Some(Ok(Notification::NewSession(_ctx))) = member3_rx.recv().await {}
    });

    let m2_sink = member2_messages.clone();
    tokio::spawn(async move {
        while let Some(Ok(Notification::NewSession(ctx))) = member2_rx.recv().await {
            collect_messages(ctx, m2_sink.clone());
        }
    });

    session
        .invite_participant(&member1_name)
        .await
        .expect("invite member1 failed")
        .await
        .expect("invite member1 completion failed");

    session
        .invite_participant(&member2_name)
        .await
        .expect("invite member2 failed")
        .await
        .expect("invite member2 completion failed");

    // 6. Baseline: member2 receives a message.
    session
        .publish_with_flags(
            &channel,
            SlimHeaderFlags::new(10, None, None, None, None),
            b"before-disconnect".to_vec(),
            None,
            None,
        )
        .await
        .expect("baseline publish failed");
    wait_for_message(
        &member2_messages,
        "before-disconnect",
        Duration::from_secs(10),
    )
    .await;

    // 7. Member2 crashes (process restart without graceful close). The moderator
    //    still considers member2 ONLINE; with MLS enabled the moderator will
    //    accept the subsequent RejoinRequest from the restored instance because
    //    on_rejoin_request processes online participants when an epoch mismatch
    //    is detected (see session_moderator.rs: on_rejoin_request).
    drop(member2_app);
    drop(member2_svc);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 8. Moderator invites member3 — the MLS epoch advances while member2 is down.
    //    Ignore the completion result: member2 is offline so its GroupUpdate ACK
    //    never arrives and the handle may time out, but the commit was sent and
    //    member3 joined; member2 will catch up via the rejoin flow on reconnect.
    let _ = session
        .invite_participant(&member3_name)
        .await
        .expect("invite member3 send failed")
        .await;

    // 9. Restore member2 from persistence. Its MLS state has the old epoch.
    let (member2_app2, _member2_rx2, conn2, _member2_svc2) = create_persistent_app(
        port,
        &member2_name,
        SharedSecret::new("member2p", TEST_VALID_SECRET).unwrap(),
        member2_dir.path(),
    )
    .await;

    let restored = member2_app2
        .restore_sessions(conn2)
        .await
        .expect("restore_sessions failed");
    assert_eq!(restored.len(), 1, "exactly one session should be restored");

    let m2_sink2 = member2_messages.clone();
    collect_messages(restored.into_iter().next().unwrap(), m2_sink2);

    // Re-establish route to moderator so RejoinRequest can be delivered.
    member2_app2
        .set_route(&moderator_name, conn2)
        .await
        .expect("set_route to moderator failed");

    // Give re-subscription time to propagate.
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // 10. Keep publishing every second. Member2 cannot decrypt messages that were
    //     encrypted with the new epoch until the heartbeat-triggered rejoin
    //     completes. wait_for_any_message polls every 50 ms for up to 30 s, which
    //     comfortably covers the 10 s heartbeat interval + rejoin round-trip.
    let session_clone = session.clone();
    let channel_clone = channel.clone();
    let publisher = tokio::spawn(async move {
        let mut i: u32 = 0;
        loop {
            i += 1;
            let msg = format!("post-restore-{i}");
            let _ = session_clone
                .publish_with_flags(
                    &channel_clone,
                    SlimHeaderFlags::new(10, None, None, None, None),
                    msg.into_bytes(),
                    None,
                    None,
                )
                .await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    wait_for_any_message(
        &member2_messages,
        |m| m.starts_with("post-restore-"),
        Duration::from_secs(30),
    )
    .await;

    publisher.abort();
}
