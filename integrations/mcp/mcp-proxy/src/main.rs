
use agp_gw::config;
//use tracing::{info, error, trace};
use agp_datapath::messages::AgentType;

mod proxy;

#[tokio::main]
async fn main() {
    // init AGP app
    // TODO get config file form command line
    let config = config::load_config("../../../data-plane/config/base/client-config.yaml").expect("failed to load configuration");
    let svc_id = agp_config::component::id::ID::new_with_str("gateway/0").unwrap();
    let _guard = config.tracing.setup_tracing_subscriber();

    let mut proxy = proxy::Proxy::new(AgentType::from_strings("cisco","mcp", "proxy"), None, config, svc_id, "http://localhost:8000/sse".to_string()).await;
    proxy.start().await;
}