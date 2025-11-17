// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, time::Duration};

use clap::Parser;
use slim::config;
use tracing::{error, info};

use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::{Name, utils::SlimHeaderFlags};
use slim_session::{Notification, SessionConfig};

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
        short,
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
        short,
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
    #[arg(short, long, value_name = "MAX_PACKETS", required = false)]
    max_packets: Option<u64>,
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
            .parse::<u64>()
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
    let mut participants = vec![];

    let msg_payload_str = if is_moderator {
        "Hello from the moderator. msg id: ".to_owned()
    } else {
        format!("Hello from the participant {}. msg id: ", local_name_str)
    };

    // start local app
    // get service
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();
    let svc_id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    // parse local name string
    let local_name = parse_string_name(local_name_str.clone());

    let channel_name = Name::from_strings(["channel", "channel", "channel"]);

    let (app, mut rx) = svc
        .create_app(
            &local_name,
            SharedSecret::new(&local_name_str, slim_testing::utils::TEST_VALID_SECRET),
            SharedSecret::new(&local_name_str, slim_testing::utils::TEST_VALID_SECRET),
        )
        .expect("failed to create app");

    // run the service - this will create all the connections provided via the config file.
    svc.run().await.unwrap();

    // get the connection id
    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .unwrap();
    info!("remote connection id = {}", conn_id);

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

    if is_moderator {
        // create session
        let config = SessionConfig {
            session_type: slim_datapath::api::ProtoSessionType::Multicast,
            max_retries: Some(10),
            interval: Some(Duration::from_secs(1)),
            mls_enabled,
            initiator: true,
            metadata: HashMap::new(),
        };
        let (session_ctx, completion_handle) = app
            .create_session(config, channel_name.clone(), Some(12345))
            .await
            .expect("error creating session");

        completion_handle.await.expect("error establishing session");

        // invite all participants
        for p in participants {
            info!("Invite participant {}", p);
            session_ctx
                .session_arc()
                .unwrap()
                .invite_participant(&p)
                .await
                .expect("error sending invite message");
        }

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
                                    info!("received message: {}", payload);
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
            info!("moderator: send message {}", i);
            // create payload
            let mut pstr = msg_payload_str.clone();
            pstr.push_str(&i.to_string());
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
        if moderator_name.is_empty() && !is_moderator {
            panic!("missing moderator name in the configuration")
        }
        let moderator = parse_string_name(moderator_name);

        // listen for sessions and messages
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
                            let moderator_clone = moderator.clone();
                            let channel_name_clone = channel_name.clone();
                            let msg_payload_str_clone = msg_payload_str.clone();
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
                                            info!("received message: {}", payload);
                                            if publisher == moderator_clone {
                                                info!("reply to moderator with message {}", msg_id);
                                                let mut pstr = msg_payload_str_clone.clone();
                                                pstr.push_str(&msg_id.to_string());
                                                let p = pstr.as_bytes().to_vec();
                                                let flags = SlimHeaderFlags::new(10, None, None, None, None);
                                                if let Some(session_arc) = weak.upgrade()
                                                    && session_arc
                                                        .publish_with_flags(&channel_name_clone, flags, p, None, None)
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
