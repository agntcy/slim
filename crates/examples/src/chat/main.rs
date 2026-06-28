// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Interactive native group-chat participant built on the **session layer** with
//! end-to-end **MLS** encryption — the same primitives the browser/wasm client
//! uses, so native and browser participants interoperate on one encrypted group.
//!
//! Two roles:
//!   * **moderator** (`--moderator`): creates the Multicast+MLS session, invites
//!     the listed participants, then chats.
//!   * **participant** (default): waits for the moderator's invite (a
//!     `NewSession` notification), then chats.
//!
//! Identity is a shared secret (`TEST_VALID_SECRET`); every participant must use
//! the same secret so MLS credential verification succeeds.
//!
//! Usage (participant):
//!   chat --config config/websocket/client-config.yaml --name org/default/native-ws/1 \
//!        --moderator-name org/default/browser/0
//! Usage (moderator):
//!   chat --config config/base/client-config.yaml --name org/default/mod/0 --moderator \
//!        --invite org/default/native-ws org/default/native-grpc
//!
//! Type a line + Enter to broadcast; received messages print as `[source] text`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::io::{AsyncBufReadExt, BufReader};

use slim::config;
use slim_auth::shared_secret::SharedSecret;
use slim_datapath::api::proto::dataplane::v1::content::ContentType;
use slim_datapath::api::{MessageType, ProtoMessage, ProtoName, ProtoSessionType};
use slim_datapath::messages::utils::SlimHeaderFlags;
use slim_session::Notification;
use slim_session::session_config::{MlsSettings, SessionConfig};
use slim_session::session_controller::SessionController;

/// Any fanout > 1 makes the data plane `match_all` (broadcast to every group
/// member except the sender).
const BROADCAST_FANOUT: u32 = 10;

#[derive(Parser)]
#[command(about = "Interactive native SLIM group chat with MLS encryption")]
struct Args {
    /// SLIM config file (defines the upstream connection, ws:// or grpc).
    #[arg(long)]
    config: String,

    /// This participant's name as `org/ns/type/id`.
    #[arg(long, default_value = "org/default/native/0")]
    name: String,

    /// The MLS group/channel name as `org/ns/name`. Must match the moderator.
    #[arg(long, default_value = "channel/channel/channel")]
    channel: String,

    /// Run as the moderator: create the MLS group and invite participants.
    #[arg(long, default_value_t = false)]
    moderator: bool,

    /// Participants to invite (moderator only), each as `org/ns/type`.
    #[arg(long, num_args = 1.., value_delimiter = ' ')]
    invite: Vec<String>,

    /// Moderator's name as `org/ns/type/id` (participant only).
    #[arg(long, default_value = "")]
    moderator_name: String,

    /// Disable MLS encryption (plaintext group).
    #[arg(long, default_value_t = false)]
    no_mls: bool,
}

/// Parse a 4-component `org/ns/type/id` name.
fn parse_name(s: &str) -> Result<ProtoName> {
    let parts: Vec<&str> = s.split('/').collect();
    anyhow::ensure!(parts.len() == 4, "name must be 'org/ns/type/id', got '{s}'");
    let id = parts[3]
        .parse::<u128>()
        .with_context(|| format!("id component must be a number, got '{}'", parts[3]))?;
    Ok(ProtoName::from_strings([
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
    ])
    .with_id(id))
}

/// Parse a 3-component `org/ns/type` name (channels and invite targets).
fn parse_type(s: &str) -> Result<ProtoName> {
    let parts: Vec<&str> = s.split('/').collect();
    anyhow::ensure!(parts.len() == 3, "type must be 'org/ns/type', got '{s}'");
    Ok(ProtoName::from_strings([
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
    ]))
}

fn source_label(msg: &ProtoMessage) -> String {
    let name = msg.get_source();
    match &name.str_name {
        Some(s) => format!(
            "{}/{}/{}",
            s.str_component_0, s.str_component_1, s.str_component_2
        ),
        None => "<unknown>".to_string(),
    }
}

fn publish_text(msg: &ProtoMessage) -> Option<String> {
    if let MessageType::Publish(p) = msg.get_type() {
        if let Some(content) = p.msg.as_ref() {
            if let Some(ContentType::AppPayload(app)) = content.content_type.as_ref() {
                return Some(String::from_utf8_lossy(&app.blob).into_owned());
            }
        }
    }
    None
}

/// Read stdin lines and broadcast each one into the (encrypted) session.
async fn stdin_publish_loop(session: Arc<SessionController>, channel: ProtoName) -> Result<()> {
    eprintln!("type a message and press Enter (Ctrl-D to quit):");
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    while let Some(line) = lines.next_line().await? {
        if line.is_empty() {
            continue;
        }
        let flags = SlimHeaderFlags::new(BROADCAST_FANOUT, None, None, None, None);
        if let Err(e) = session
            .publish_with_flags(&channel, flags, line.into_bytes(), None, None)
            .await
        {
            eprintln!("send error: {e}");
        }
    }
    eprintln!("stdin closed, exiting");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let local_name = parse_name(&args.name)?;
    let channel = parse_type(&args.channel)?;

    // Load the service from the config file (defines the upstream connection).
    let mut loader = config::ConfigLoader::new(&args.config).context("failed to load config")?;
    let _guard = loader
        .tracing()
        .context("invalid tracing configuration")?
        .setup_tracing_subscriber();

    let svc_id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let services = loader.services().context("failed to load services")?;
    let svc = services.get_mut(&svc_id).unwrap();

    // Identity: a shared secret used as both token provider and verifier. Every
    // participant must use the same secret for MLS credential verification.
    let (app, mut rx) = svc
        .create_app(
            &local_name,
            SharedSecret::new(&args.name, slim_testing::utils::TEST_VALID_SECRET)
                .expect("failed to create SharedSecret"),
            SharedSecret::new(&args.name, slim_testing::utils::TEST_VALID_SECRET)
                .expect("failed to create SharedSecret"),
        )
        .context("failed to create app")?;

    // Run the service: opens the connections defined in the config file.
    svc.run().await.context("failed to run service")?;

    let conn_id = svc
        .get_connection_id(&svc.config().dataplane_clients()[0].endpoint)
        .context("no upstream connection")?;
    eprintln!("upstream connection established (conn {conn_id})");

    // Subscribe so the node routes messages addressed to us back down.
    app.subscribe(&local_name, Some(conn_id))
        .await
        .context("subscribe failed")?;

    let mls_settings = if args.no_mls {
        None
    } else {
        Some(MlsSettings::default())
    };

    if args.moderator {
        // ── Moderator: create the MLS group, invite participants, then chat ──
        let invitees: Vec<ProtoName> = args
            .invite
            .iter()
            .map(|s| parse_type(s))
            .collect::<Result<_>>()?;
        anyhow::ensure!(
            !invitees.is_empty(),
            "moderator needs at least one --invite participant"
        );

        // Route invites/messages toward each participant via the upstream node.
        for p in &invitees {
            app.set_route(p, conn_id).await.context("set_route failed")?;
        }
        // Give the routes a moment to propagate.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let session_config = SessionConfig {
            session_type: ProtoSessionType::Multicast,
            max_retries: Some(10),
            interval: Some(Duration::from_secs(1)),
            mls_settings,
            initiator: true,
            metadata: HashMap::new(),
        };
        let (session_ctx, completion) = app
            .create_session(session_config, channel.clone(), Some(12345))
            .await
            .context("create_session failed")?;
        completion.await.context("session init failed")?;

        let session = session_ctx.session_arc().context("session dropped")?;

        for p in &invitees {
            eprintln!("inviting {p}");
            session
                .invite_participant(p)
                .await
                .with_context(|| format!("invite {p} failed"))?;
        }
        eprintln!(
            "MLS group '{}' ready{}",
            args.channel,
            if args.no_mls { " (MLS disabled)" } else { "" }
        );

        // Print inbound messages from the group.
        session_ctx.spawn_receiver(|mut rx, _weak| async move {
            while let Some(item) = rx.recv().await {
                match item {
                    Ok(msg) => {
                        if let Some(text) = publish_text(&msg) {
                            println!("[{}] {}", source_label(&msg), text);
                        }
                    }
                    Err(e) => {
                        eprintln!("recv error: {e}");
                        break;
                    }
                }
            }
        });

        stdin_publish_loop(session, channel).await?;
    } else {
        // ── Participant: wait for the moderator's invite, then chat ──────────
        if !args.moderator_name.is_empty() {
            eprintln!(
                "waiting for invite from moderator {} ...",
                args.moderator_name
            );
        } else {
            eprintln!("waiting for invite from a moderator ...");
        }

        loop {
            match rx.recv().await {
                None => {
                    eprintln!("notification stream closed");
                    break;
                }
                Some(Ok(Notification::NewSession(session_ctx))) => {
                    eprintln!("joined MLS group '{}'", args.channel);
                    let session = session_ctx.session_arc().context("session dropped")?;

                    session_ctx.spawn_receiver(|mut rx, _weak| async move {
                        while let Some(item) = rx.recv().await {
                            match item {
                                Ok(msg) => {
                                    if let Some(text) = publish_text(&msg) {
                                        println!("[{}] {}", source_label(&msg), text);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("recv error: {e}");
                                    break;
                                }
                            }
                        }
                    });

                    stdin_publish_loop(session, channel.clone()).await?;
                    break;
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => {
                    eprintln!("notification error: {e}");
                    continue;
                }
            }
        }
    }

    Ok(())
}
