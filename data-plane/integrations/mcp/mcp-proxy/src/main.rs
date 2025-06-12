// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use slim::config;
use slim_datapath::messages::AgentType;
use tracing::{error, info};

mod proxy;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// SLIM configuration file
    #[arg(short, long, value_name = "configuration", required = true)]
    config: String,

    /// Service name to look for in the configuration file
    #[arg(short, long, value_name = "svc_name", required = true)]
    svc_name: String,

    /// MCP Proxy name in the form org/ns/type
    #[arg(short, long, value_name = "proxy_name", required = true)]
    name: String,

    /// MCP Proxy instance ID
    #[arg(short, long, value_name = "id", required = false)]
    id: Option<u64>,

    /// MCP Server address (e.g http://localhost:8000/sse)
    #[arg(short, long, value_name = "address", required = true)]
    mcp_server: String,
}

impl Args {
    pub fn config(&self) -> &String {
        &self.config
    }

    pub fn svc_name(&self) -> &String {
        &self.svc_name
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn id(&self) -> Option<&u64> {
        self.id.as_ref()
    }

    pub fn mcp_server(&self) -> &String {
        &self.mcp_server
    }
}

#[tokio::main]
async fn main() {
    // parse command line
    let args = Args::parse();

    let config_file = args.config();
    let svc_name = args.svc_name();
    let name = args.name();
    let id = args.id();
    let server = args.mcp_server();

    let v_name: Vec<&str> = name.split('/').collect();
    if v_name.len() != 3 {
        error!("error processing the MCP proxy name, invalid format");
        return;
    }

    let config = config::load_config(config_file).expect("failed to load configuration");
    let svc_id = slim_config::component::id::ID::new_with_str(svc_name).unwrap();
    let _guard = config.tracing.setup_tracing_subscriber();

    let mut proxy = proxy::Proxy::new(
        AgentType::from_strings(v_name[0], v_name[1], v_name[2]),
        None,
        id.copied(),
        config,
        svc_id,
        server.clone(),
    );

    info!("starting MCP proxy");
    proxy.start().await;
}
