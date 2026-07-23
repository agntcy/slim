// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, time::Duration};

use clap::Parser;
use slim::config;
use slim_datapath::api::ProtoName as Name;
use tracing::{error, info};

use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::utils::SlimHeaderFlags;
use slim_session::Notification;
use slim_session::session_config::{MlsSettings, SessionConfig};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Slim config file
    #[arg(short, long, value_name = "CONFIGURATION", required = true)]
    config: String,

    /// Local endpoint name in the form org/ns/type/id
    #[arg(short, long, value_name = "ENDOPOINT", required = true)]
    name: String,

    /// Runs the endpoint in moderator mode.
    #[arg(
        short,
        long,
        value_name = "IS_MODERATOR",
        required = false,
        default_value_t = false
    )]
    is_moderator: bool,

    /// Runs the endpoint with MLS disabled.
    #[arg(
        short = 'd',
        long,
        value_name = "MSL_DISABLED",
        required = false,
        default_value_t = false
    )]
    mls_disabled: bool,

    // List of participants types to add to the channel in the form org/ns/type. used only in moderator mode
    #[clap(short, long, value_name = "PARTICIPANTS", num_args = 1.., value_delimiter = ' ', required = false)]
    participants: Vec<String>,

    // Moderator name in the for org/ns/type/id. used only in participant mode
    #[arg(
        short = 'o',
        long,
        value_name = "MODERATOR_NAME",
        required = false,
        default_value = ""
    )]
    moderator_name: String,

    /// Time between publications in milliseconds
    #[arg(
        short,
        long,
        value_name = "FREQUENCY",
        required = false,
        default_value_t = 1000
    )]
    frequency: u32,

    /// Maximum number of packets to send. used only by the moderator
    #[arg(short = 'm', long, value_name = "MAX_PACKETS", required = false)]
    max_packets: Option<u64>,

    /// Directory for encrypted, restorable session state. When set, the endpoint
    /// persists its MLS/session state here and restores it on restart (so it can
    /// be killed and relaunched without repeating the invite/welcome handshake).
    #[arg(long, value_name = "PERSISTENCE_DIR", required = false)]
    persistence_dir: Option<String>,
}

impl Args {
    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn config(&self) -> &String {
        &self.config
    }

    pub fn is_moderator(&self) -> &bool {
        &self.is_moderator
    }

    pub fn mls_disabled(&self) -> &bool {
        &self.mls_disabled
    }

    pub fn moderator_name(&self) -> &String {
        &self.moderator_name
    }

    pub fn participants(&self) -> &Vec<String> {
        &self.participants
    }

    pub fn frequency(&self) -> &u32 {
        &self.frequency
    }

    pub fn max_packets(&self) -> &Option<u64> {
        &self.max_packets
    }

    pub fn persistence_dir(&self) -> &Option<String> {
        &self.persistence_dir
    }
}

fn parse_string_name(name: String) -> Name {
    let mut strs = name.split('/');
    Name::from_strings([
        strs.next().expect("error parsing local_name string"),
        strs.next().expect("error parsing local_name string"),
        strs.next().expect("error parsing local_name string"),
    ])
    .with_id(
        strs.next()
            .expect("error parsing local_name string")
            .parse::<u128>()
            .expect("error parsing local_name string"),
    )
}

fn parse_string_type(name: String) -> Name {
    let mut strs = name.split('/');
    Name::from_strings([
        strs.next().expect("error parsing local_name string"),
        strs.next().expect("error parsing local_name string"),
        strs.next().expect("error parsing local_name string"),
    ])
}

/// Spawn the participant-side receiver on a session: log received messages and
/// reply to the moderator. Shared by freshly-joined sessions and sessions
/// restored from persistence.
fn spawn_participant_receiver(
    session_ctx: slim_session::context::SessionContext,
    moderator: Name,
    channel_name: Name,
    msg_payload_str: String,
) {
    session_ctx.spawn_receiver(move |mut rx, weak| async move {
        loop {
            match rx.recv().await {
                None => {
                    println!("Session receiver: end of stream");
                    break;
                }
                Some(Ok(msg)) => {
                    let publisher = msg.get_slim_header().get_source();
                    let msg_id = msg.get_id();
                    let payload = if let Some(slim_datapath::api::ProtoPublishType(publish)) =
                        msg.message_type.as_ref()
                    {
                        let blob = &publish.get_payload().as_application_payload().unwrap().blob;
                        match String::from_utf8(blob.to_vec()) {
                            Ok(p) => p,
                            Err(e) => {
                                error!("error while parsing the message {}", e.to_string());
                                String::new()
                            }
                        }
                    } else {
                        String::new()
                    };
                    info!(%payload, "received message");
                    if publisher == moderator {
                        info!(%msg_id, "reply to moderator");
                        let mut pstr = msg_payload_str.clone();
                        pstr.push_str(&msg_id.to_string());
                        let p = pstr.as_bytes().to_vec();
                        let flags = SlimHeaderFlags::new(10, None, None, None, None);
                        if let Some(session_arc) = weak.upgrade()
                            && session_arc
                                .publish_with_flags(&channel_name, flags, p, None, None)
                                .await
                                .is_err()
                        {
                            panic!("an error occurred sending publication from moderator");
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

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let config_file = args.config();
    let local_name_str = args.name().clone();
    let frequency = *args.frequency();
    let is_moderator = *args.is_moderator();
    let mls_enabled = !*args.mls_disabled();
    let moderator_name = args.moderator_name().clone();
    let max_packets = args.max_packets;
    let participants_str = args.participants().clone();
    let persistence_dir = args.persistence_dir().clone();
    let mut participants = vec![];

    let msg_payload_str = if is_moderator {
        "Hello from the moderator. msg id: ".to_owned()
    } else {
        format!("Hello from the participant {}. msg id: ", local_name_str)
    };

    // start local app
    // get service
    let mut config = config::ConfigLoader::new(config_file).expect("failed to load configuration");
    let _guard = config
        .tracing()
        .expect("invalid tracing configuration")
        .setup_tracing_subscriber();
    let svc_id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let svc = config
        .services()
        .expect("failed to load services")
        .get_mut(&svc_id)
        .unwrap();

    // parse local name string
    let local_name = parse_string_name(local_name_str.clone());

    let channel_name = Name::from_strings(["channel", "channel", "channel"]);

    let provider = SharedSecret::new(&local_name_str, slim_testing::utils::TEST_VALID_SECRET)
        .expect("Failed to create SharedSecret");
    let verifier = SharedSecret::new(&local_name_str, slim_testing::utils::TEST_VALID_SECRET)
        .expect("Failed to create SharedSecret");

    // Enable persistence when --persistence-dir is set, so the app's MLS/session
    // state survives a restart and can be restored below.
    let (app, mut rx) = match &persistence_dir {
        Some(dir) => {
            info!(dir, "persistence enabled");
            svc.create_app_with_direction_and_persistence(
                &local_name,
                provider,
                verifier,
                slim_session::Direction::Bidirectional,
                Some(slim_persistence::PersistenceConfig::new(dir)),
            )
            .expect("failed to create app")
        }
        None => svc
            .create_app(&local_name, provider, verifier)
            .expect("failed to create app"),
    };

    // run the service - this will create all the connections provided via the config file.
    svc.run().await.unwrap();

    // get the connection id
    let conn_id = svc
        .get_connection_id(&svc.config().dataplane_clients()[0].endpoint)
        .unwrap();
    info!(remote_connection_id = %conn_id);

    // subscribe for local name
    app.subscribe(&local_name, Some(conn_id))
        .await
        .expect("an error accoured while adding a subscription");

    if is_moderator {
        if participants_str.is_empty() {
            panic!("the participant list is missing.");
        }

        for n in participants_str {
            // add to the participants list
            let p = parse_string_type(n);
            participants.push(p.clone());

            // add route
            app.set_route(&p, conn_id)
                .await
                .expect("an error accoured while adding a route");
        }
    }

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Restore any persisted sessions (no-op when persistence is disabled). A
    // restored endpoint rejoins its existing MLS group instead of starting a
    // fresh invite/welcome handshake.
    let mut restored = if persistence_dir.is_some() {
        let sessions = app
            .restore_sessions(conn_id)
            .await
            .expect("error restoring persisted sessions");
        info!(count = sessions.len(), "restored persisted sessions");
        sessions
    } else {
        Vec::new()
    };

    if is_moderator {
        let session_ctx = if let Some(ctx) = restored.pop() {
            info!("resuming moderator session from persistence (skipping create + invite)");
            ctx
        } else {
            // create session
            let config = SessionConfig {
                session_type: slim_datapath::api::ProtoSessionType::Multicast,
                max_retries: Some(10),
                interval: Some(Duration::from_secs(1)),
                mls_settings: if mls_enabled {
                    Some(MlsSettings::default())
                } else {
                    None
                },
                initiator: true,
                metadata: HashMap::new(),
            };
            let (ctx, completion_handle) = app
                .create_session(config, channel_name.clone(), Some(12345))
                .await
                .expect("error creating session");

            completion_handle.await.expect("error establishing session");

            // invite all participants
            for p in participants {
                info!(participant = %p, "Invite participant");
                ctx.session_arc()
                    .unwrap()
                    .invite_participant(&p)
                    .await
                    .expect("error sending invite message");
            }
            ctx
        };

        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

        let session_arc = session_ctx.session_arc().unwrap();
        // listen for messages
        session_ctx.spawn_receiver(move |mut rx, _weak| async move {
            loop {
                match rx.recv().await {
                    None => {
                        info!(%conn_id, "end of stream");
                        break;
                    }
                    Some(message) => match message {
                        Ok(msg) => {
                            if let Some(slim_datapath::api::ProtoPublishType(publish)) =
                                msg.message_type.as_ref()
                            {
                                let p =
                                    &publish.get_payload().as_application_payload().unwrap().blob;
                                if let Ok(payload) = String::from_utf8(p.to_vec()) {
                                    info!(%payload, "received message");
                                }
                            }
                        }
                        Err(e) => {
                            error!("received an error message {}", e);
                        }
                    },
                }
            }
        });

        for i in 0..max_packets.unwrap_or(u64::MAX) {
            // create payload
            let mut pstr = msg_payload_str.clone();
            pstr.push_str(&i.to_string());
            info!(payload = %pstr, "moderator: send message");
            let p = pstr.as_bytes().to_vec();

            // set fanout > 1 to send the message in broadcast
            let flags = SlimHeaderFlags::new(10, None, None, None, None);

            if session_arc
                .publish_with_flags(&channel_name, flags, p, None, None)
                .await
                .is_err()
            {
                panic!("an error occurred sending publication from moderator",);
            }
            if frequency != 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(frequency as u64)).await;
            }
        }
    } else {
        // participant
        if moderator_name.is_empty() {
            panic!("missing moderator name in the configuration")
        }
        let moderator = parse_string_name(moderator_name);

        // Resume any sessions restored from persistence right away.
        for session_ctx in restored {
            info!("resuming participant session from persistence");
            spawn_participant_receiver(
                session_ctx,
                moderator.clone(),
                channel_name.clone(),
                msg_payload_str.clone(),
            );
        }

        // listen for new sessions and messages
        loop {
            match rx.recv().await {
                None => {
                    info!(%conn_id, "end of stream");
                    break;
                }
                Some(res) => match res {
                    Ok(notification) => match notification {
                        Notification::NewSession(session_ctx) => {
                            println!("received a new session");
                            spawn_participant_receiver(
                                session_ctx,
                                moderator.clone(),
                                channel_name.clone(),
                                msg_payload_str.clone(),
                            );
                        }
                        _ => {
                            println!("Unexpected notification type");
                            continue;
                        }
                    },
                    Err(e) => {
                        println!("received an error message {}", e);
                        continue;
                    }
                },
            }
        }
    }
}
