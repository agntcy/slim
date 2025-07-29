// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use clap::Parser;
use slim::config;
use tracing::{error, info};

use slim_auth::shared_secret::SharedSecret;
use slim_datapath::messages::{Agent, AgentType, utils::SlimHeaderFlags};
use slim_service::streaming::StreamingConfiguration;

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

    /// Runs the endpoint in attacker mode.
    #[arg(
        short,
        long,
        value_name = "IS_ATTACKER",
        required = false,
        default_value_t = false
    )]
    is_attacker: bool,

    /// Runs the endpoint with MLS disabled.
    #[arg(
        short,
        long,
        value_name = "MSL_DISABLED",
        required = false,
        default_value_t = false
    )]
    mls_disabled: bool,

    // List of paticipants types to add to the channel in the form org/ns/type. used only in moderator mode
    #[clap(short, long, value_name = "PARITICIPANTS", num_args = 1.., value_delimiter = ' ', required = false)]
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

    pub fn is_attacker(&self) -> &bool {
        &self.is_attacker
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

fn parse_string_name(name: String) -> Agent {
    let mut strs = name.split('/');
    Agent::from_strings(
        strs.next().expect("error parsing local_name string"),
        strs.next().expect("error parsing local_name string"),
        strs.next().expect("error parsing local_name string"),
        strs.next()
            .expect("error parsing local_name string")
            .parse::<u64>()
            .expect("error parsing local_name string"),
    )
}

fn parse_string_type(name: String) -> AgentType {
    let mut strs = name.split('/');
    AgentType::from_strings(
        strs.next().expect("error parsing local_name string"),
        strs.next().expect("error parsing local_name string"),
        strs.next().expect("error parsing local_name string"),
    )
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let config_file = args.config();
    let local_name_str = args.name().clone();
    let frequency = *args.frequency();
    let is_moderator = *args.is_moderator();
    let is_attacker = *args.is_attacker();
    let msl_enabled = !*args.mls_disabled();
    let moderator_name = args.moderator_name().clone();
    let max_packets = args.max_packets;
    let participants_str = args.participants().clone();
    let mut participants = vec![];

    let msg_payload_str = if is_moderator {
        "Hello from the moderator. msg id: ".to_owned()
    } else {
        format!("Hello from the participant {}. msg id: ", local_name_str)
    };

    // start local agent
    // get service
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();
    let svc_id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    // parse local name string
    let local_name = parse_string_name(local_name_str.clone());

    let channel_name = AgentType::from_strings("channel", "channel", "channel");

    let (app, mut rx) = svc
        .create_app(
            &local_name,
            SharedSecret::new(&local_name_str, "group"),
            SharedSecret::new(&local_name_str, "group"),
        )
        .await
        .expect("failed to create agent");

    // run the service - this will create all the connections provided via the config file.
    svc.run().await.unwrap();

    // get the connection id
    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .unwrap();
    info!("remote connection id = {}", conn_id);

    // subscribe for local name
    app.subscribe(
        local_name.agent_type(),
        local_name.agent_id_option(),
        Some(conn_id),
    )
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
            app.set_route(&p, None, conn_id)
                .await
                .expect("an error accoured while adding a route");
        }
    }

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    if is_moderator {
        // create session
        let info = app
            .create_session(
                slim_service::session::SessionConfig::Streaming(StreamingConfiguration::new(
                    slim_service::session::SessionDirection::Bidirectional,
                    Some(channel_name.clone()),
                    true,
                    Some(10),
                    Some(Duration::from_secs(1)),
                    msl_enabled,
                )),
                Some(12345),
            )
            .await
            .expect("error creating session");

        // invite all participants
        for p in participants {
            info!("Invite participant {}", p);
            app.invite_participant(&p, info.clone())
                .await
                .expect("error sending invite message");
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

        // listen for messages
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    None => {
                        info!(%conn_id, "end of stream");
                        break;
                    }
                    Some(msg_info) => match msg_info {
                        Ok(msg) => {
                            let payload = match msg.message.get_payload() {
                                Some(c) => {
                                    let p = &c.blob;
                                    String::from_utf8(p.to_vec())
                                        .expect("error while parsing received message")
                                }
                                None => "".to_string(),
                            };

                            info!("received message: {}", payload);
                        }
                        Err(e) => {
                            error!("received an error message {:?}", e);
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

            if app
                .publish_with_flags(info.clone(), &channel_name, None, flags, p)
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

        if is_attacker {
            info!("Starting the attacker");
            let _ = app
                .create_session(
                    slim_service::session::SessionConfig::Streaming(StreamingConfiguration::new(
                        slim_service::session::SessionDirection::Bidirectional,
                        Some(channel_name.clone()),
                        true,
                        Some(10),
                        Some(Duration::from_secs(1)),
                        true,
                    )),
                    Some(12345),
                )
                .await
                .expect("error creating session");

            // subscribe for local name
            app.subscribe(&channel_name, None, Some(conn_id))
                .await
                .expect("an error accoured while adding a subscription");
        }

        // listen for messages
        loop {
            match rx.recv().await {
                None => {
                    info!(%conn_id, "end of stream");
                    break;
                }
                Some(msg_info) => match msg_info {
                    Ok(msg) => {
                        let publisher = msg.message.get_slim_header().get_source();
                        let msg_id = msg.message.get_id();
                        let payload = match msg.message.get_payload() {
                            Some(c) => {
                                let blob = &c.blob;
                                match String::from_utf8(blob.to_vec()) {
                                    Ok(p) => p,
                                    Err(e) => {
                                        error!("error while parsing the message {}", e.to_string());
                                        continue;
                                    }
                                }
                            }
                            None => "".to_string(),
                        };

                        let info = msg.info;
                        info!("received message: {}", payload,);

                        if publisher == moderator && !is_attacker {
                            // for each message coming from the moderator reply with another message
                            info!("reply to moderator with message {}", msg_id);

                            // create payload
                            let mut pstr = msg_payload_str.clone();
                            pstr.push_str(&msg_id.to_string());
                            let p = pstr.as_bytes().to_vec();

                            let flags = SlimHeaderFlags::new(10, None, None, None, None);
                            if app
                                .publish_with_flags(info, &channel_name, None, flags, p)
                                .await
                                .is_err()
                            {
                                panic!("an error occurred sending publication from moderator",);
                            }
                        }
                    }
                    Err(e) => {
                        error!("received an error message {:?}", e);
                    }
                },
            }
        }
    }
}
