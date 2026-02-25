// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use crate::commands::{
    config_cmd::{self, ConfigArgs},
    controller::{self, ControllerArgs},
    node::{self, NodeArgs},
    slim_cmd::{self, SlimArgs},
    version,
};
use crate::config::{ResolvedOpts, load_config};

/// SLIM control CLI
#[derive(Parser)]
#[command(name = "slimctl", about = "SLIM control CLI")]
pub(crate) struct Cli {
    #[command(flatten)]
    global: GlobalOpts,

    #[command(subcommand)]
    command: Commands,
}

/// Global options applied to all commands (may appear before or after the subcommand)
#[derive(Args)]
struct GlobalOpts {
    /// Path to slimctl configuration file
    #[arg(long, env = "SLIMCTL_CONFIG", global = true)]
    config: Option<String>,

    /// Basic auth credentials (username:password)
    #[arg(
        short = 'b',
        long,
        env = "SLIMCTL_COMMON_OPTS_BASIC_AUTH_CREDS",
        global = true
    )]
    basic_auth_creds: Option<String>,

    /// SLIM gRPC control API endpoint (host:port)
    #[arg(short = 's', long, env = "SLIMCTL_COMMON_OPTS_SERVER", global = true)]
    server: Option<String>,

    /// gRPC request timeout (e.g. 15s, 1m)
    #[arg(long, env = "SLIMCTL_COMMON_OPTS_TIMEOUT", global = true)]
    timeout: Option<String>,

    /// Disable TLS (plain HTTP/2)
    #[arg(
        long = "tls.insecure",
        env = "SLIMCTL_COMMON_OPTS_TLS_INSECURE",
        global = true
    )]
    tls_insecure: bool,

    /// Use TLS but skip server certificate verification
    #[arg(
        long = "tls.insecure_skip_verify",
        env = "SLIMCTL_COMMON_OPTS_TLS_INSECURE_SKIP_VERIFY",
        global = true
    )]
    tls_insecure_skip_verify: bool,

    /// Path to TLS CA certificate
    #[arg(
        long = "tls.ca_file",
        env = "SLIMCTL_COMMON_OPTS_TLS_CA_FILE",
        global = true
    )]
    tls_ca_file: Option<String>,

    /// Path to client TLS certificate
    #[arg(
        long = "tls.cert_file",
        env = "SLIMCTL_COMMON_OPTS_TLS_CERT_FILE",
        global = true
    )]
    tls_cert_file: Option<String>,

    /// Path to client TLS key
    #[arg(
        long = "tls.key_file",
        env = "SLIMCTL_COMMON_OPTS_TLS_KEY_FILE",
        global = true
    )]
    tls_key_file: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Print version information
    Version,

    /// Manage slimctl configuration
    Config(ConfigArgs),

    /// Commands to interact with SLIM nodes directly
    #[command(aliases = ["n", "instance", "i"])]
    Node(NodeArgs),

    /// Commands to interact with the SLIM Control Plane
    #[command(aliases = ["c", "ctrl"])]
    Controller(ControllerArgs),

    /// Commands for managing a local SLIM instance
    #[command(alias = "s")]
    Slim(SlimArgs),
}

pub(crate) async fn run(cli: Cli) -> Result<()> {
    // Initialize the TLS crypto provider before any operation that may open a
    // gRPC/TLS connection.  This must happen exactly once per process and must
    // precede any rustls call.
    slim_config::tls::provider::initialize_crypto_provider();

    // Load config file, then overlay with CLI/env opts
    let file_config = load_config(cli.global.config.as_deref())?;
    let opts = ResolvedOpts::resolve(
        &file_config,
        cli.global.server.as_deref(),
        cli.global.timeout.as_deref(),
        cli.global.tls_insecure,
        cli.global.tls_insecure_skip_verify,
        cli.global.tls_ca_file.as_deref(),
        cli.global.tls_cert_file.as_deref(),
        cli.global.tls_key_file.as_deref(),
        cli.global.basic_auth_creds.as_deref(),
    )?;

    match cli.command {
        Commands::Version => {
            version::run();
        }
        Commands::Config(args) => {
            config_cmd::run(&args, cli.global.config.as_deref()).await?;
        }
        Commands::Node(args) => {
            node::run(&args, &opts).await?;
        }
        Commands::Controller(args) => {
            controller::run(&args, &opts).await?;
        }
        Commands::Slim(args) => {
            slim_cmd::run(&args).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::config_cmd::{ConfigCommand, SetCommand};
    use crate::commands::controller::{
        ControllerChannelCommand, ControllerCommand, ControllerConnectionCommand,
        ControllerNodeCommand, ControllerParticipantCommand, ControllerRouteCommand,
    };
    use crate::commands::node::{NodeCommand, NodeConnectionCommand, NodeRouteCommand};
    use crate::commands::slim_cmd::SlimCommand;
    use crate::config::HOME_LOCK;

    /// Parse args and return the `Cli`, panicking with a helpful message on error.
    fn parse_ok(args: &[&str]) -> Cli {
        Cli::try_parse_from(args)
            .unwrap_or_else(|e| panic!("expected parse success for {args:?}, got: {e}"))
    }

    /// Parse args and return the clap error (expects failure).
    fn parse_err(args: &[&str]) -> clap::Error {
        match Cli::try_parse_from(args) {
            Err(e) => e,
            Ok(_) => panic!("expected parse failure for {args:?}"),
        }
    }

    /// Point HOME at a fresh temp directory for the duration of the caller's scope.
    #[allow(clippy::disallowed_methods)]
    fn setup_home() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by HOME_LOCK held by the caller.
        unsafe { std::env::set_var("HOME", dir.path()) };
        dir
    }

    // ── version ──────────────────────────────────────────────────────────────

    #[test]
    fn parse_version() {
        let cli = parse_ok(&["slimctl", "version"]);
        assert!(matches!(cli.command, Commands::Version));
    }

    #[test]
    fn global_flag_config() {
        let cli = parse_ok(&["slimctl", "--config", "/etc/slimctl/config.yaml", "version"]);
        assert_eq!(
            cli.global.config.as_deref(),
            Some("/etc/slimctl/config.yaml")
        );
    }

    #[test]
    fn global_flag_config_absent_is_none() {
        let cli = parse_ok(&["slimctl", "version"]);
        assert!(cli.global.config.is_none());
    }

    // ── config ───────────────────────────────────────────────────────────────

    #[test]
    fn parse_config_list() {
        let cli = parse_ok(&["slimctl", "config", "list"]);
        let Commands::Config(args) = cli.command else {
            panic!()
        };
        assert!(matches!(args.command, ConfigCommand::List));
    }

    #[test]
    fn parse_config_list_alias_ls() {
        let cli = parse_ok(&["slimctl", "config", "ls"]);
        let Commands::Config(args) = cli.command else {
            panic!()
        };
        assert!(matches!(args.command, ConfigCommand::List));
    }

    #[test]
    fn parse_config_set_server() {
        let cli = parse_ok(&["slimctl", "config", "set", "server", "myhost:9090"]);
        let Commands::Config(args) = cli.command else {
            panic!()
        };
        let ConfigCommand::Set(s) = args.command else {
            panic!()
        };
        let SetCommand::Server { value } = s.command else {
            panic!()
        };
        assert_eq!(value, "myhost:9090");
    }

    #[test]
    fn parse_config_set_timeout() {
        let cli = parse_ok(&["slimctl", "config", "set", "timeout", "30s"]);
        let Commands::Config(args) = cli.command else {
            panic!()
        };
        let ConfigCommand::Set(s) = args.command else {
            panic!()
        };
        let SetCommand::Timeout { value } = s.command else {
            panic!()
        };
        assert_eq!(value, "30s");
    }

    #[test]
    fn parse_config_set_tls_insecure() {
        let cli = parse_ok(&["slimctl", "config", "set", "tls-insecure", "false"]);
        let Commands::Config(args) = cli.command else {
            panic!()
        };
        let ConfigCommand::Set(s) = args.command else {
            panic!()
        };
        let SetCommand::TlsInsecure { value } = s.command else {
            panic!()
        };
        assert_eq!(value, "false");
    }

    #[test]
    fn parse_config_set_tls_insecure_skip_verify() {
        let cli = parse_ok(&[
            "slimctl",
            "config",
            "set",
            "tls-insecure-skip-verify",
            "true",
        ]);
        let Commands::Config(args) = cli.command else {
            panic!()
        };
        let ConfigCommand::Set(s) = args.command else {
            panic!()
        };
        let SetCommand::TlsInsecureSkipVerify { value } = s.command else {
            panic!()
        };
        assert_eq!(value, "true");
    }

    #[test]
    fn parse_config_set_basic_auth_creds() {
        let cli = parse_ok(&["slimctl", "config", "set", "basic-auth-creds", "user:pass"]);
        let Commands::Config(args) = cli.command else {
            panic!()
        };
        let ConfigCommand::Set(s) = args.command else {
            panic!()
        };
        let SetCommand::BasicAuthCreds { value } = s.command else {
            panic!()
        };
        assert_eq!(value, "user:pass");
    }

    #[test]
    fn parse_config_set_tls_ca_file() {
        let cli = parse_ok(&["slimctl", "config", "set", "tls-ca-file", "/etc/ca.pem"]);
        let Commands::Config(args) = cli.command else {
            panic!()
        };
        let ConfigCommand::Set(s) = args.command else {
            panic!()
        };
        let SetCommand::TlsCaFile { value } = s.command else {
            panic!()
        };
        assert_eq!(value, "/etc/ca.pem");
    }

    // ── controller ────────────────────────────────────────────────────────────

    #[test]
    fn parse_controller_node_list() {
        let cli = parse_ok(&["slimctl", "controller", "node", "list"]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Node(a) = args.command else {
            panic!()
        };
        assert!(matches!(a.command, ControllerNodeCommand::List));
    }

    #[test]
    fn parse_controller_connection_list() {
        let cli = parse_ok(&["slimctl", "controller", "connection", "list", "-n", "node1"]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Connection(a) = args.command else {
            panic!()
        };
        let ControllerConnectionCommand::List { node_id } = a.command;
        assert_eq!(node_id, "node1");
    }

    #[test]
    fn parse_controller_route_list() {
        let cli = parse_ok(&["slimctl", "controller", "route", "list", "-n", "node1"]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Route(a) = args.command else {
            panic!()
        };
        let ControllerRouteCommand::List { node_id } = a.command else {
            panic!()
        };
        assert_eq!(node_id, "node1");
    }

    #[test]
    fn parse_controller_route_add() {
        let cli = parse_ok(&[
            "slimctl",
            "controller",
            "route",
            "add",
            "-n",
            "node1",
            "org/ns/agent/42",
            "via",
            "dest-node",
        ]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Route(a) = args.command else {
            panic!()
        };
        let ControllerRouteCommand::Add {
            node_id,
            route,
            via,
            destination,
        } = a.command
        else {
            panic!()
        };
        assert_eq!(node_id, "node1");
        assert_eq!(route, "org/ns/agent/42");
        assert_eq!(via, "via");
        assert_eq!(destination, "dest-node");
    }

    #[test]
    fn parse_controller_route_del() {
        let cli = parse_ok(&[
            "slimctl",
            "controller",
            "route",
            "del",
            "-n",
            "node1",
            "org/ns/agent/42",
            "via",
            "http://host:8080",
        ]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Route(a) = args.command else {
            panic!()
        };
        let ControllerRouteCommand::Del {
            node_id,
            route,
            via,
            destination,
        } = a.command
        else {
            panic!()
        };
        assert_eq!(node_id, "node1");
        assert_eq!(route, "org/ns/agent/42");
        assert_eq!(via, "via");
        assert_eq!(destination, "http://host:8080");
    }

    #[test]
    fn parse_controller_route_outline_defaults() {
        let cli = parse_ok(&["slimctl", "controller", "route", "outline"]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Route(a) = args.command else {
            panic!()
        };
        let ControllerRouteCommand::Outline {
            origin_node_id,
            target_node_id,
        } = a.command
        else {
            panic!()
        };
        assert_eq!(origin_node_id, "");
        assert_eq!(target_node_id, "");
    }

    #[test]
    fn parse_controller_route_outline_with_filters() {
        let cli = parse_ok(&[
            "slimctl",
            "controller",
            "route",
            "outline",
            "-o",
            "src-node",
            "-t",
            "dst-node",
        ]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Route(a) = args.command else {
            panic!()
        };
        let ControllerRouteCommand::Outline {
            origin_node_id,
            target_node_id,
        } = a.command
        else {
            panic!()
        };
        assert_eq!(origin_node_id, "src-node");
        assert_eq!(target_node_id, "dst-node");
    }

    #[test]
    fn parse_controller_channel_create() {
        let cli = parse_ok(&[
            "slimctl",
            "controller",
            "channel",
            "create",
            "moderators=alice,bob",
        ]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Channel(a) = args.command else {
            panic!()
        };
        let ControllerChannelCommand::Create { moderators_param } = a.command else {
            panic!()
        };
        assert_eq!(moderators_param, "moderators=alice,bob");
    }

    #[test]
    fn parse_controller_channel_delete() {
        let cli = parse_ok(&["slimctl", "controller", "channel", "delete", "chan1"]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Channel(a) = args.command else {
            panic!()
        };
        let ControllerChannelCommand::Delete { channel_name } = a.command else {
            panic!()
        };
        assert_eq!(channel_name, "chan1");
    }

    #[test]
    fn parse_controller_channel_list() {
        let cli = parse_ok(&["slimctl", "controller", "channel", "list"]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Channel(a) = args.command else {
            panic!()
        };
        assert!(matches!(a.command, ControllerChannelCommand::List));
    }

    #[test]
    fn parse_controller_participant_add() {
        let cli = parse_ok(&[
            "slimctl",
            "controller",
            "participant",
            "add",
            "alice",
            "-c",
            "chan1",
        ]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Participant(a) = args.command else {
            panic!()
        };
        let ControllerParticipantCommand::Add {
            participant_name,
            channel_id,
        } = a.command
        else {
            panic!()
        };
        assert_eq!(participant_name, "alice");
        assert_eq!(channel_id, "chan1");
    }

    #[test]
    fn parse_controller_participant_delete() {
        let cli = parse_ok(&[
            "slimctl",
            "controller",
            "participant",
            "delete",
            "alice",
            "-c",
            "chan1",
        ]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Participant(a) = args.command else {
            panic!()
        };
        let ControllerParticipantCommand::Delete {
            participant_name,
            channel_id,
        } = a.command
        else {
            panic!()
        };
        assert_eq!(participant_name, "alice");
        assert_eq!(channel_id, "chan1");
    }

    #[test]
    fn parse_controller_participant_list() {
        let cli = parse_ok(&[
            "slimctl",
            "controller",
            "participant",
            "list",
            "-c",
            "chan1",
        ]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Participant(a) = args.command else {
            panic!()
        };
        let ControllerParticipantCommand::List { channel_id } = a.command else {
            panic!()
        };
        assert_eq!(channel_id, "chan1");
    }

    // ── node ─────────────────────────────────────────────────────────────────

    #[test]
    fn parse_node_route_list() {
        let cli = parse_ok(&["slimctl", "node", "route", "list"]);
        let Commands::Node(args) = cli.command else {
            panic!()
        };
        let NodeCommand::Route(a) = args.command else {
            panic!()
        };
        assert!(matches!(a.command, NodeRouteCommand::List));
    }

    #[test]
    fn parse_node_route_add() {
        let cli = parse_ok(&[
            "slimctl",
            "node",
            "route",
            "add",
            "org/ns/agent/42",
            "via",
            "/path/to/config.json",
        ]);
        let Commands::Node(args) = cli.command else {
            panic!()
        };
        let NodeCommand::Route(a) = args.command else {
            panic!()
        };
        let NodeRouteCommand::Add {
            route,
            via,
            config_file,
        } = a.command
        else {
            panic!()
        };
        assert_eq!(route, "org/ns/agent/42");
        assert_eq!(via, "via");
        assert_eq!(config_file, "/path/to/config.json");
    }

    #[test]
    fn parse_node_route_del() {
        let cli = parse_ok(&[
            "slimctl",
            "node",
            "route",
            "del",
            "org/ns/agent/42",
            "via",
            "http://host:8080",
        ]);
        let Commands::Node(args) = cli.command else {
            panic!()
        };
        let NodeCommand::Route(a) = args.command else {
            panic!()
        };
        let NodeRouteCommand::Del {
            route,
            via,
            endpoint,
        } = a.command
        else {
            panic!()
        };
        assert_eq!(route, "org/ns/agent/42");
        assert_eq!(via, "via");
        assert_eq!(endpoint, "http://host:8080");
    }

    #[test]
    fn parse_node_connection_list() {
        let cli = parse_ok(&["slimctl", "node", "connection", "list"]);
        let Commands::Node(args) = cli.command else {
            panic!()
        };
        let NodeCommand::Connection(a) = args.command else {
            panic!()
        };
        assert!(matches!(a.command, NodeConnectionCommand::List));
    }

    // ── slim ──────────────────────────────────────────────────────────────────

    #[test]
    fn parse_slim_start_with_config() {
        let cli = parse_ok(&["slimctl", "slim", "start", "-c", "/etc/slim/config.yaml"]);
        let Commands::Slim(args) = cli.command else {
            panic!()
        };
        let SlimCommand::Start(start_args) = args.command;
        assert_eq!(start_args.config.as_deref(), Some("/etc/slim/config.yaml"));
    }

    #[test]
    fn parse_slim_start_with_endpoint() {
        let cli = parse_ok(&["slimctl", "slim", "start", "--endpoint", "0.0.0.0:46357"]);
        let Commands::Slim(args) = cli.command else {
            panic!()
        };
        let SlimCommand::Start(start_args) = args.command;
        assert_eq!(start_args.endpoint.as_deref(), Some("0.0.0.0:46357"));
    }

    // ── command aliases ───────────────────────────────────────────────────────

    #[test]
    fn alias_node_n() {
        let cli = parse_ok(&["slimctl", "n", "route", "list"]);
        assert!(matches!(cli.command, Commands::Node(_)));
    }

    #[test]
    fn alias_node_instance() {
        let cli = parse_ok(&["slimctl", "instance", "route", "list"]);
        assert!(matches!(cli.command, Commands::Node(_)));
    }

    #[test]
    fn alias_node_i() {
        let cli = parse_ok(&["slimctl", "i", "route", "list"]);
        assert!(matches!(cli.command, Commands::Node(_)));
    }

    #[test]
    fn alias_controller_c() {
        let cli = parse_ok(&["slimctl", "c", "node", "list"]);
        assert!(matches!(cli.command, Commands::Controller(_)));
    }

    #[test]
    fn alias_controller_ctrl() {
        let cli = parse_ok(&["slimctl", "ctrl", "node", "list"]);
        assert!(matches!(cli.command, Commands::Controller(_)));
    }

    #[test]
    fn alias_slim_s() {
        let cli = parse_ok(&["slimctl", "s", "start"]);
        assert!(matches!(cli.command, Commands::Slim(_)));
    }

    #[test]
    fn alias_controller_route_ls() {
        let cli = parse_ok(&["slimctl", "controller", "route", "ls", "-n", "node1"]);
        let Commands::Controller(args) = cli.command else {
            panic!()
        };
        let ControllerCommand::Route(a) = args.command else {
            panic!()
        };
        assert!(matches!(a.command, ControllerRouteCommand::List { .. }));
    }

    #[test]
    fn alias_node_connection_conn() {
        let cli = parse_ok(&["slimctl", "node", "conn", "list"]);
        let Commands::Node(args) = cli.command else {
            panic!()
        };
        assert!(matches!(args.command, NodeCommand::Connection(_)));
    }

    #[test]
    fn alias_node_route_ls() {
        let cli = parse_ok(&["slimctl", "node", "route", "ls"]);
        let Commands::Node(args) = cli.command else {
            panic!()
        };
        let NodeCommand::Route(a) = args.command else {
            panic!()
        };
        assert!(matches!(a.command, NodeRouteCommand::List));
    }

    // ── global flags ──────────────────────────────────────────────────────────

    #[test]
    fn global_flag_server_long() {
        let cli = parse_ok(&["slimctl", "--server", "custom:9090", "version"]);
        assert_eq!(cli.global.server.as_deref(), Some("custom:9090"));
    }

    #[test]
    fn global_flag_server_short() {
        let cli = parse_ok(&["slimctl", "-s", "custom:9090", "version"]);
        assert_eq!(cli.global.server.as_deref(), Some("custom:9090"));
    }

    #[test]
    fn global_flag_timeout() {
        let cli = parse_ok(&["slimctl", "--timeout", "30s", "version"]);
        assert_eq!(cli.global.timeout.as_deref(), Some("30s"));
    }

    #[test]
    fn global_flag_tls_insecure() {
        let cli = parse_ok(&["slimctl", "--tls.insecure", "version"]);
        assert!(cli.global.tls_insecure);
    }

    #[test]
    fn global_flag_tls_insecure_skip_verify() {
        let cli = parse_ok(&["slimctl", "--tls.insecure_skip_verify", "version"]);
        assert!(cli.global.tls_insecure_skip_verify);
    }

    #[test]
    fn global_flag_tls_ca_file() {
        let cli = parse_ok(&["slimctl", "--tls.ca_file", "/etc/ca.pem", "version"]);
        assert_eq!(cli.global.tls_ca_file.as_deref(), Some("/etc/ca.pem"));
    }

    #[test]
    fn global_flag_tls_cert_and_key() {
        let cli = parse_ok(&[
            "slimctl",
            "--tls.cert_file",
            "/etc/cert.pem",
            "--tls.key_file",
            "/etc/key.pem",
            "version",
        ]);
        assert_eq!(cli.global.tls_cert_file.as_deref(), Some("/etc/cert.pem"));
        assert_eq!(cli.global.tls_key_file.as_deref(), Some("/etc/key.pem"));
    }

    #[test]
    fn global_flag_basic_auth_creds() {
        let cli = parse_ok(&["slimctl", "--basic-auth-creds", "user:pass", "version"]);
        assert_eq!(cli.global.basic_auth_creds.as_deref(), Some("user:pass"));
    }

    #[test]
    fn global_flags_absent_are_none_or_false() {
        let cli = parse_ok(&["slimctl", "version"]);
        assert!(cli.global.config.is_none());
        assert!(cli.global.server.is_none());
        assert!(cli.global.timeout.is_none());
        assert!(!cli.global.tls_insecure);
        assert!(!cli.global.tls_insecure_skip_verify);
        assert!(cli.global.tls_ca_file.is_none());
        assert!(cli.global.tls_cert_file.is_none());
        assert!(cli.global.tls_key_file.is_none());
        assert!(cli.global.basic_auth_creds.is_none());
    }

    // ── invalid / missing arguments ───────────────────────────────────────────

    #[test]
    fn unknown_top_level_command_fails() {
        let err = parse_err(&["slimctl", "unknown-command"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn missing_subcommand_fails() {
        // clap emits DisplayHelpOnMissingArgumentOrSubcommand when a required
        // subcommand is absent, so assert failure rather than a specific kind.
        parse_err(&["slimctl", "controller"]);
    }

    #[test]
    fn controller_connection_list_missing_node_id_fails() {
        let err = parse_err(&["slimctl", "controller", "connection", "list"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn controller_route_list_missing_node_id_fails() {
        let err = parse_err(&["slimctl", "controller", "route", "list"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn controller_participant_list_missing_channel_id_fails() {
        let err = parse_err(&["slimctl", "controller", "participant", "list"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn controller_channel_create_missing_moderators_arg_fails() {
        let err = parse_err(&["slimctl", "controller", "channel", "create"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn config_set_missing_value_fails() {
        let err = parse_err(&["slimctl", "config", "set", "server"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    // ── run-level tests (commands that don't need gRPC) ───────────────────────

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn cmd_version_runs_successfully() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = setup_home();
        run(parse_ok(&["slimctl", "version"])).await.unwrap();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn cmd_config_list_runs_successfully() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = setup_home();
        run(parse_ok(&["slimctl", "config", "list"])).await.unwrap();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn cmd_config_set_server_persists() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = setup_home();
        run(parse_ok(&[
            "slimctl",
            "config",
            "set",
            "server",
            "myhost:9090",
        ]))
        .await
        .unwrap();
        assert_eq!(
            crate::config::load_config(None).unwrap().common_opts.server,
            "myhost:9090"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn cmd_config_set_timeout_persists() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = setup_home();
        run(parse_ok(&["slimctl", "config", "set", "timeout", "2m"]))
            .await
            .unwrap();
        assert_eq!(
            crate::config::load_config(None)
                .unwrap()
                .common_opts
                .timeout,
            "2m"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn cmd_config_set_invalid_timeout_fails() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = setup_home();
        assert!(
            run(parse_ok(&[
                "slimctl",
                "config",
                "set",
                "timeout",
                "notaduration"
            ]))
            .await
            .is_err()
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn cmd_config_set_tls_insecure_true_persists() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = setup_home();
        run(parse_ok(&[
            "slimctl",
            "config",
            "set",
            "tls-insecure",
            "true",
        ]))
        .await
        .unwrap();
        assert!(
            crate::config::load_config(None)
                .unwrap()
                .common_opts
                .tls_insecure
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn cmd_config_set_tls_insecure_invalid_value_fails() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = setup_home();
        assert!(
            run(parse_ok(&[
                "slimctl",
                "config",
                "set",
                "tls-insecure",
                "maybe"
            ]))
            .await
            .is_err()
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn cmd_config_set_tls_insecure_skip_verify_true_persists() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = setup_home();
        run(parse_ok(&[
            "slimctl",
            "config",
            "set",
            "tls-insecure-skip-verify",
            "true",
        ]))
        .await
        .unwrap();
        assert!(
            crate::config::load_config(None)
                .unwrap()
                .common_opts
                .tls_insecure_skip_verify
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn cmd_config_set_tls_insecure_skip_verify_invalid_value_fails() {
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = setup_home();
        assert!(
            run(parse_ok(&[
                "slimctl",
                "config",
                "set",
                "tls-insecure-skip-verify",
                "maybe"
            ]))
            .await
            .is_err()
        );
    }

    #[test]
    fn parse_slim_start_no_args_uses_defaults() {
        // Neither --config nor --endpoint: both are None; default endpoint is applied at runtime
        let cli = parse_ok(&["slimctl", "slim", "start"]);
        let Commands::Slim(args) = cli.command else {
            panic!()
        };
        let SlimCommand::Start(start_args) = args.command;
        assert!(start_args.config.is_none());
        assert!(start_args.endpoint.is_none());
    }

    #[test]
    fn parse_slim_start_config_and_endpoint_together_fails() {
        let err = parse_err(&[
            "slimctl",
            "slim",
            "start",
            "--config",
            "foo.yaml",
            "--endpoint",
            "0.0.0.0:46357",
        ]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}
