//! Shared utilities for SLIM black-box integration tests.

mod command;
mod config;
mod log;
mod ports;
mod process;
mod routing;

pub use command::run_combined_output_with_retry;
pub use config::{new_temp_dir, write_temp_config, write_temp_config_near_source};
pub use log::{ProcessLogWatcher, contains_semver_fragment, wait_for_log};
pub use ports::reserve_port;
pub use process::{session_still_running, terminate_session};
pub use routing::{
    run_slimctl_controller_retry, run_slimctl_node_add_route_via, run_slimctl_node_output,
    run_slimctl_node_retry, wait_for_route_with_connections_format,
    wait_for_route_with_uuid_suffix,
};
