use anyhow::Result;
use clap::Parser;

mod cli;
mod config;
mod controller_cmds;
mod defaults;
mod node_connection;
mod node_route;
mod proto_gen;
mod slim_start;
mod version;

use cli::{
    Cli, Commands, ConfigSetKey, ConfigSubcommand, ControllerChannelSubcommand,
    ControllerConnectionSubcommand, ControllerParticipantSubcommand, ControllerRouteSubcommand,
    ControllerSubcommand, NodeConnectionSubcommand, NodeRouteSubcommand, NodeSubcommand,
    SlimSubcommand,
};
use config::{AppConfig, CliCommonOverrides};

fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli)
}

fn run(cli: Cli) -> Result<()> {
    let file_config = config::load_first_existing_config()?;
    let common_overrides = CliCommonOverrides {
        basic_auth_creds: cli.basic_auth_creds.clone(),
        server: cli.server.clone(),
        timeout: cli.timeout.clone(),
        tls_insecure: cli.tls_insecure,
        tls_ca_file: cli.tls_ca_file.clone(),
        tls_cert_file: cli.tls_cert_file.clone(),
        tls_key_file: cli.tls_key_file.clone(),
    };
    let effective_opts = config::resolve_effective_opts(&file_config, &common_overrides);

    match cli.command {
        Commands::Version => version::print_version(),
        Commands::Config(config_cmd) => match config_cmd.command {
            ConfigSubcommand::List => {
                let persisted = config::load_home_config()?;
                let output = serde_yaml::to_string(&persisted)?;
                println!("{output}");
            }
            ConfigSubcommand::Set(set_cmd) => {
                let mut current = config::load_home_config()?;
                match set_cmd.key {
                    ConfigSetKey::BasicAuthCreds { value } => {
                        current.common_opts.basic_auth_creds = Some(value)
                    }
                    ConfigSetKey::Server { value } => current.common_opts.server = Some(value),
                    ConfigSetKey::Timeout { value } => current.common_opts.timeout = Some(value),
                    ConfigSetKey::TlsCaFile { value } => {
                        current.common_opts.tls_ca_file = Some(value)
                    }
                    ConfigSetKey::TlsCertFile { value } => {
                        current.common_opts.tls_cert_file = Some(value)
                    }
                    ConfigSetKey::TlsKeyFile { value } => {
                        current.common_opts.tls_key_file = Some(value)
                    }
                    ConfigSetKey::TlsInsecure { value } => {
                        current.common_opts.tls_insecure = Some(value)
                    }
                }

                let path = config::save_home_config(&current)?;
                println!("Saved config to {}", path.display());
            }
        },
        Commands::Controller(controller) => match controller.command {
            ControllerSubcommand::Node(node) => match node.command {
                cli::ControllerNodeSubcommand::List => {
                    controller_cmds::list_nodes(&effective_opts.server)?
                }
            },
            ControllerSubcommand::Connection(connection) => match connection.command {
                ControllerConnectionSubcommand::List { node_id } => {
                    controller_cmds::list_connections(&effective_opts.server, &node_id)?
                }
            },
            ControllerSubcommand::Route(route) => match route.command {
                ControllerRouteSubcommand::List { node_id } => {
                    controller_cmds::list_routes(&effective_opts.server, &node_id)?
                }
                ControllerRouteSubcommand::Add {
                    node_id,
                    route,
                    via_keyword,
                    destination,
                } => controller_cmds::add_route(
                    &effective_opts.server,
                    &node_id,
                    &route,
                    &via_keyword,
                    &destination,
                )?,
                ControllerRouteSubcommand::Del {
                    node_id,
                    route,
                    via_keyword,
                    destination,
                } => controller_cmds::del_route(
                    &effective_opts.server,
                    &node_id,
                    &route,
                    &via_keyword,
                    &destination,
                )?,
                ControllerRouteSubcommand::Outline {
                    origin_node_id,
                    target_node_id,
                } => controller_cmds::outline_routes(
                    &effective_opts.server,
                    origin_node_id,
                    target_node_id,
                )?,
            },
            ControllerSubcommand::Channel(channel) => match channel.command {
                ControllerChannelSubcommand::Create {
                    moderators_assignment,
                } => {
                    controller_cmds::create_channel(&effective_opts.server, &moderators_assignment)?
                }
                ControllerChannelSubcommand::Delete { channel_id } => {
                    controller_cmds::delete_channel(&effective_opts.server, &channel_id)?
                }
                ControllerChannelSubcommand::List => {
                    controller_cmds::list_channels(&effective_opts.server)?
                }
            },
            ControllerSubcommand::Participant(participant) => match participant.command {
                ControllerParticipantSubcommand::Add {
                    channel_id,
                    participant_name,
                } => controller_cmds::add_participant(
                    &effective_opts.server,
                    &channel_id,
                    &participant_name,
                )?,
                ControllerParticipantSubcommand::Delete {
                    channel_id,
                    participant_name,
                } => controller_cmds::delete_participant(
                    &effective_opts.server,
                    &channel_id,
                    &participant_name,
                )?,
                ControllerParticipantSubcommand::List { channel_id } => {
                    controller_cmds::list_participants(&effective_opts.server, &channel_id)?
                }
            },
        },
        Commands::Node(node) => match node.command {
            NodeSubcommand::Route(route) => match route.command {
                NodeRouteSubcommand::List => node_route::route_list(&effective_opts.server)?,
                NodeRouteSubcommand::Add {
                    route,
                    via_keyword,
                    config_file,
                } => node_route::route_add(
                    &route,
                    &via_keyword,
                    &config_file,
                    &effective_opts.server,
                )?,
                NodeRouteSubcommand::Del {
                    route,
                    via_keyword,
                    endpoint,
                } => {
                    node_route::route_del(&route, &via_keyword, &endpoint, &effective_opts.server)?
                }
            },
            NodeSubcommand::Connection(connection) => match connection.command {
                NodeConnectionSubcommand::List => {
                    node_connection::connection_list(&effective_opts.server)?
                }
            },
        },
        Commands::Slim(slim_cmd) => match slim_cmd.command {
            SlimSubcommand::Start { config, endpoint } => {
                slim_start::start(config, endpoint)?;
            }
        },
    }

    Ok(())
}

#[allow(dead_code)]
fn _render_effective_config_for_debug(config: &AppConfig) -> Result<String> {
    Ok(serde_yaml::to_string(config)?)
}

#[cfg(test)]
mod tests {
    use super::_render_effective_config_for_debug;
    use super::cli::{
        ConfigCommand, ConfigSubcommand, ControllerChannelCommand, ControllerChannelSubcommand,
        ControllerCommand, ControllerConnectionCommand, ControllerConnectionSubcommand,
        ControllerNodeCommand, ControllerNodeSubcommand, ControllerParticipantCommand,
        ControllerParticipantSubcommand, ControllerRouteCommand, ControllerRouteSubcommand,
        ControllerSubcommand, NodeCommand, NodeConnectionCommand, NodeConnectionSubcommand,
        NodeRouteCommand, NodeRouteSubcommand, NodeSubcommand, SlimCommand, SlimSubcommand,
    };
    use super::{Cli, Commands, config::AppConfig, run};

    #[test]
    fn render_effective_config_outputs_yaml() {
        let config = AppConfig::default();
        let rendered =
            _render_effective_config_for_debug(&config).expect("should render config to yaml");
        assert!(rendered.contains("common_opts"));
    }

    fn base_cli(command: Commands) -> Cli {
        Cli {
            basic_auth_creds: None,
            server: None,
            timeout: None,
            tls_insecure: None,
            tls_ca_file: None,
            tls_cert_file: None,
            tls_key_file: None,
            command,
        }
    }

    #[test]
    fn run_handles_version() {
        let cli = base_cli(Commands::Version);
        run(cli).expect("version should run");
    }

    #[test]
    fn run_handles_config_list() {
        let cli = base_cli(Commands::Config(ConfigCommand {
            command: ConfigSubcommand::List,
        }));
        run(cli).expect("config list should run");
    }

    #[test]
    fn run_handles_controller_subcommands() {
        let cli = base_cli(Commands::Controller(ControllerCommand {
            command: ControllerSubcommand::Node(ControllerNodeCommand {
                command: ControllerNodeSubcommand::List,
            }),
        }));
        run(cli).expect("controller node list should run");

        let cli = base_cli(Commands::Controller(ControllerCommand {
            command: ControllerSubcommand::Connection(ControllerConnectionCommand {
                command: ControllerConnectionSubcommand::List {
                    node_id: "node-1".to_string(),
                },
            }),
        }));
        run(cli).expect("controller connection list should run");

        let cli = base_cli(Commands::Controller(ControllerCommand {
            command: ControllerSubcommand::Route(ControllerRouteCommand {
                command: ControllerRouteSubcommand::List {
                    node_id: "node-1".to_string(),
                },
            }),
        }));
        run(cli).expect("controller route list should run");

        let cli = base_cli(Commands::Controller(ControllerCommand {
            command: ControllerSubcommand::Route(ControllerRouteCommand {
                command: ControllerRouteSubcommand::Add {
                    node_id: "node-1".to_string(),
                    route: "org/ns/app/1".to_string(),
                    via_keyword: "via".to_string(),
                    destination: "node-2".to_string(),
                },
            }),
        }));
        run(cli).expect("controller route add should run");

        let cli = base_cli(Commands::Controller(ControllerCommand {
            command: ControllerSubcommand::Route(ControllerRouteCommand {
                command: ControllerRouteSubcommand::Del {
                    node_id: "node-1".to_string(),
                    route: "org/ns/app/1".to_string(),
                    via_keyword: "via".to_string(),
                    destination: "node-2".to_string(),
                },
            }),
        }));
        run(cli).expect("controller route del should run");

        let cli = base_cli(Commands::Controller(ControllerCommand {
            command: ControllerSubcommand::Route(ControllerRouteCommand {
                command: ControllerRouteSubcommand::Outline {
                    origin_node_id: None,
                    target_node_id: None,
                },
            }),
        }));
        run(cli).expect("controller route outline should run");

        let cli = base_cli(Commands::Controller(ControllerCommand {
            command: ControllerSubcommand::Channel(ControllerChannelCommand {
                command: ControllerChannelSubcommand::Create {
                    moderators_assignment: "moderators=a".to_string(),
                },
            }),
        }));
        run(cli).expect("controller channel create should run");

        let cli = base_cli(Commands::Controller(ControllerCommand {
            command: ControllerSubcommand::Channel(ControllerChannelCommand {
                command: ControllerChannelSubcommand::Delete {
                    channel_id: "channel".to_string(),
                },
            }),
        }));
        run(cli).expect("controller channel delete should run");

        let cli = base_cli(Commands::Controller(ControllerCommand {
            command: ControllerSubcommand::Channel(ControllerChannelCommand {
                command: ControllerChannelSubcommand::List,
            }),
        }));
        run(cli).expect("controller channel list should run");

        let cli = base_cli(Commands::Controller(ControllerCommand {
            command: ControllerSubcommand::Participant(ControllerParticipantCommand {
                command: ControllerParticipantSubcommand::Add {
                    channel_id: "channel".to_string(),
                    participant_name: "participant".to_string(),
                },
            }),
        }));
        run(cli).expect("controller participant add should run");

        let cli = base_cli(Commands::Controller(ControllerCommand {
            command: ControllerSubcommand::Participant(ControllerParticipantCommand {
                command: ControllerParticipantSubcommand::Delete {
                    channel_id: "channel".to_string(),
                    participant_name: "participant".to_string(),
                },
            }),
        }));
        run(cli).expect("controller participant delete should run");

        let cli = base_cli(Commands::Controller(ControllerCommand {
            command: ControllerSubcommand::Participant(ControllerParticipantCommand {
                command: ControllerParticipantSubcommand::List {
                    channel_id: "channel".to_string(),
                },
            }),
        }));
        run(cli).expect("controller participant list should run");
    }

    #[test]
    fn run_handles_node_and_slim_commands() {
        let cli = base_cli(Commands::Node(NodeCommand {
            command: NodeSubcommand::Route(NodeRouteCommand {
                command: NodeRouteSubcommand::List,
            }),
        }));
        run(cli).expect("node route list should run");

        let cli = base_cli(Commands::Node(NodeCommand {
            command: NodeSubcommand::Route(NodeRouteCommand {
                command: NodeRouteSubcommand::Add {
                    route: "org/ns/app/1".to_string(),
                    via_keyword: "via".to_string(),
                    config_file: "config.json".to_string(),
                },
            }),
        }));
        run(cli).expect("node route add should run");

        let cli = base_cli(Commands::Node(NodeCommand {
            command: NodeSubcommand::Route(NodeRouteCommand {
                command: NodeRouteSubcommand::Del {
                    route: "org/ns/app/1".to_string(),
                    via_keyword: "via".to_string(),
                    endpoint: "http://localhost:1234".to_string(),
                },
            }),
        }));
        run(cli).expect("node route del should run");

        let cli = base_cli(Commands::Node(NodeCommand {
            command: NodeSubcommand::Connection(NodeConnectionCommand {
                command: NodeConnectionSubcommand::List,
            }),
        }));
        run(cli).expect("node connection list should run");

        let cli = base_cli(Commands::Slim(SlimCommand {
            command: SlimSubcommand::Start {
                config: "".to_string(),
                endpoint: "".to_string(),
            },
        }));
        run(cli).expect("slim start should run");
    }
}
