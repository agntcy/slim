use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "slimctl", about = "SLIM control CLI", long_about = None)]
pub struct Cli {
    #[arg(short = 'b', long = "basic_auth_creds", global = true)]
    pub basic_auth_creds: Option<String>,

    #[arg(short = 's', long = "server", global = true)]
    pub server: Option<String>,

    #[arg(long = "timeout", global = true)]
    pub timeout: Option<String>,

    #[arg(long = "tls.insecure", global = true)]
    pub tls_insecure: Option<bool>,

    #[arg(long = "tls.ca_file", global = true)]
    pub tls_ca_file: Option<String>,

    #[arg(long = "tls.cert_file", global = true)]
    pub tls_cert_file: Option<String>,

    #[arg(long = "tls.key_file", global = true)]
    pub tls_key_file: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Version,
    Config(ConfigCommand),
    #[command(alias = "c", alias = "ctrl")]
    Controller(ControllerCommand),
    #[command(alias = "n", alias = "instance", alias = "i")]
    Node(NodeCommand),
    #[command(alias = "s")]
    Slim(SlimCommand),
}

#[derive(Debug, Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: ConfigSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigSubcommand {
    #[command(alias = "ls")]
    List,
    Set(ConfigSetCommand),
}

#[derive(Debug, Args)]
pub struct ConfigSetCommand {
    #[command(subcommand)]
    pub key: ConfigSetKey,
}

#[derive(Debug, Subcommand)]
pub enum ConfigSetKey {
    #[command(name = "basic-auth-creds")]
    BasicAuthCreds {
        value: String,
    },
    Server {
        value: String,
    },
    Timeout {
        value: String,
    },
    #[command(name = "tls-ca-file")]
    TlsCaFile {
        value: String,
    },
    #[command(name = "tls-cert-file")]
    TlsCertFile {
        value: String,
    },
    #[command(name = "tls-key-file")]
    TlsKeyFile {
        value: String,
    },
    #[command(name = "tls-insecure")]
    TlsInsecure {
        value: bool,
    },
}

#[derive(Debug, Args)]
pub struct ControllerCommand {
    #[command(subcommand)]
    pub command: ControllerSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ControllerSubcommand {
    Node(ControllerNodeCommand),
    #[command(alias = "conn")]
    Connection(ControllerConnectionCommand),
    Route(ControllerRouteCommand),
    Channel(ControllerChannelCommand),
    Participant(ControllerParticipantCommand),
}

#[derive(Debug, Args)]
pub struct ControllerNodeCommand {
    #[command(subcommand)]
    pub command: ControllerNodeSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ControllerNodeSubcommand {
    #[command(alias = "ls")]
    List,
}

#[derive(Debug, Args)]
pub struct ControllerConnectionCommand {
    #[command(subcommand)]
    pub command: ControllerConnectionSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ControllerConnectionSubcommand {
    #[command(alias = "ls")]
    List {
        #[arg(short = 'n', long = "node-id", required = true)]
        node_id: String,
    },
}

#[derive(Debug, Args)]
pub struct ControllerRouteCommand {
    #[command(subcommand)]
    pub command: ControllerRouteSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ControllerRouteSubcommand {
    List {
        #[arg(short = 'n', long = "node-id", required = true)]
        node_id: String,
    },
    Add {
        #[arg(short = 'n', long = "node-id", required = true)]
        node_id: String,
        route: String,
        via_keyword: String,
        destination: String,
    },
    Del {
        #[arg(short = 'n', long = "node-id", required = true)]
        node_id: String,
        route: String,
        via_keyword: String,
        destination: String,
    },
    Outline {
        #[arg(short = 'o', long = "origin-node-id")]
        origin_node_id: Option<String>,
        #[arg(short = 't', long = "target-node-id")]
        target_node_id: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct ControllerChannelCommand {
    #[command(subcommand)]
    pub command: ControllerChannelSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ControllerChannelSubcommand {
    Create { moderators_assignment: String },
    Delete { channel_id: String },
    List,
}

#[derive(Debug, Args)]
pub struct ControllerParticipantCommand {
    #[command(subcommand)]
    pub command: ControllerParticipantSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ControllerParticipantSubcommand {
    Add {
        #[arg(short = 'c', long = "channel-id", required = true)]
        channel_id: String,
        participant_name: String,
    },
    Delete {
        #[arg(short = 'c', long = "channel-id", required = true)]
        channel_id: String,
        participant_name: String,
    },
    List {
        #[arg(short = 'c', long = "channel-id", required = true)]
        channel_id: String,
    },
}

#[derive(Debug, Args)]
pub struct NodeCommand {
    #[command(subcommand)]
    pub command: NodeSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum NodeSubcommand {
    Route(NodeRouteCommand),
    #[command(alias = "conn")]
    Connection(NodeConnectionCommand),
}

#[derive(Debug, Args)]
pub struct NodeRouteCommand {
    #[command(subcommand)]
    pub command: NodeRouteSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum NodeRouteSubcommand {
    #[command(alias = "ls")]
    List,
    Add {
        route: String,
        via_keyword: String,
        config_file: String,
    },
    Del {
        route: String,
        via_keyword: String,
        endpoint: String,
    },
}

#[derive(Debug, Args)]
pub struct NodeConnectionCommand {
    #[command(subcommand)]
    pub command: NodeConnectionSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum NodeConnectionSubcommand {
    #[command(alias = "ls")]
    List,
}

#[derive(Debug, Args)]
pub struct SlimCommand {
    #[command(subcommand)]
    pub command: SlimSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum SlimSubcommand {
    Start {
        #[arg(short = 'c', long = "config", default_value = "")]
        config: String,
        #[arg(long = "endpoint", default_value = "")]
        endpoint: String,
    },
}
