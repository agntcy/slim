// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use slim_datapath::messages::{Agent, AgentType, utils::SlimHeaderFlags};

use slim_service::streaming::StreamingConfiguration;

use clap::Parser;
use slim::config;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Slim config file
    #[arg(short, long, value_name = "CONFIGURATION", required = true)]
    config: String,

    /// endpoint id
    #[arg(short, long, value_name = "ID", required = true)]
    id: u64,

    /// Runs in pub/sub mode.
    #[arg(
        short,
        long,
        value_name = "MODERATOR",
        required = false,
        default_value_t = false
    )]
    moderator: bool,

    /// Publication message size
    #[arg(
        short,
        long,
        value_name = "SIZE",
        required = false,
        default_value_t = 1500
    )]
    msg_size: u32,

    /// time between publications in milliseconds
    #[arg(
        short,
        long,
        value_name = "FREQUENCY",
        required = false,
        default_value_t = 1000
    )]
    frequency: u32,

    /// used only in streaming mode, defines the maximum number of packets to send
    #[arg(short, long, value_name = "PACKETS", required = false)]
    max_packets: Option<u64>,
}

impl Args {
    pub fn msg_size(&self) -> &u32 {
        &self.msg_size
    }

    pub fn id(&self) -> &u64 {
        &self.id
    }

    pub fn config(&self) -> &String {
        &self.config
    }

    pub fn moderator(&self) -> &bool {
        &self.moderator
    }

    pub fn frequency(&self) -> &u32 {
        &self.frequency
    }

    pub fn max_packets(&self) -> &Option<u64> {
        &self.max_packets
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let config_file = args.config();
    let msg_size = *args.msg_size();
    let id = *args.id();
    let frequency = *args.frequency();
    let moderator = *args.moderator();
    let max_packets = args.max_packets;

    // start local agent
    // get service
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();
    let svc_id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    let participant = Agent::from_strings("org", "default", "participant", id);

    // create local agent
    let local_name = if moderator {
        Agent::from_strings("org", "default", "moderator", id)
    } else {
        // TODO this should be a list of pariticipants
        participant.clone()
    };
    let channel_name = AgentType::from_strings("topic", "topic", "topic");

    let mut rx = svc
        .create_agent(&local_name)
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
    svc.subscribe(
        &local_name,
        local_name.agent_type(),
        local_name.agent_id_option(),
        Some(conn_id),
    )
    .await
    .expect("an error accoured while adding a subscription");

    if moderator {
        svc.set_route(
            &local_name,
            participant.agent_type(),
            participant.agent_id_option(),
            conn_id,
        )
        .await
        .expect("an error accoured while adding a route");
    }

    // wait for the connection to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    if moderator {
        // create session
        let info = svc
            .create_session(
                &local_name,
                slim_service::session::SessionConfig::Streaming(StreamingConfiguration::new(
                    slim_service::session::SessionDirection::Bidirectional,
                    Some(channel_name.clone()),
                    true,
                    Some(10),
                    Some(Duration::from_secs(1)),
                )),
            )
            .await
            .expect("error creating session");

        // TODO loop over all the participants and invite all of them
        svc.send_invite_message(&local_name, participant.agent_type(), info.clone())
            .await
            .expect("error sending invite message");

        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

        for i in 0..max_packets.unwrap_or(u64::MAX) {
            let payload: Vec<u8> = vec![120; msg_size as usize]; // ASCII for 'x' = 120
            info!("publishing message {}", i);
            // set fanout > 1 to send the message in broadcast
            let flags = SlimHeaderFlags::new(10, None, None, None, None);
            if svc
                .publish_with_flags(
                    &local_name,
                    info.clone(),
                    &channel_name,
                    None,
                    flags,
                    payload,
                )
                .await
                .is_err()
            {
                error!("an error occurred sending publication, the test will fail",);
            }
            if frequency != 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(frequency as u64)).await;
            }
        }
    } else {
        // listen for messages
        // TODO: one the session is established start to send messages
        //tokio::spawn(async move {
        loop {
            match rx.recv().await {
                None => {
                    info!(%conn_id, "end of stream");
                    break;
                }
                Some(msg_info) => match msg_info {
                    Ok(msg) => {
                        let publisher_id = msg.message.get_slim_header().get_source().agent_id();
                        info!(
                            "received message {} from publisher {}",
                            msg.info.message_id.unwrap(),
                            publisher_id
                        );
                    }
                    Err(e) => {
                        error!("received an error message {:?}", e);
                    }
                },
            }
        }
        //});
    }
}
