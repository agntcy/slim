// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Path to the config file
    #[arg(short, long, value_name = "FILE")]
    #[clap(long, env, required = true)]
    config: String,

    /// Set the topic to subscribe to.
    #[clap(long, env, required = true)]
    #[arg(short, long, value_name = "LOCAL_NAME")]
    local_name: String,

    /// Set the topic to subscribe to.
    #[clap(long, env, required = true)]
    #[arg(short, long, value_name = "REMOTE_NAME")]
    remote_name: String,

    /// Set the message to publish. If not set, the program will subscribe to the topic.
    #[clap(long, env)]
    #[arg(short, long, value_name = "MESSAGE")]
    message: Option<String>,

    /// MLS group identifier.
    #[clap(long, env)]
    #[arg(long, value_name = "MLS_GROUP_ID")]
    mls_group_id: Option<String>,

    /// Override the dataplane `forward_to` connection index for the initial self-subscribe.
    ///
    /// When unset, the first configured dataplane client endpoint is used (single-hop setups).
    /// On a relay that already has an upstream federation link, this must match that link's
    /// connection id on the **dataplane** node (often `0` when the uplink is established first).
    #[clap(long, env)]
    #[arg(long, value_name = "CONN_INDEX")]
    dataplane_forward_conn: Option<u64>,

    /// Override the dataplane `recv_from` connection index used for `set_route` towards the remote app.
    ///
    /// When unset, the first configured dataplane client endpoint is used. For a node that
    /// accepts local apps on a different gRPC connection than the federation uplink, set this
    /// to the local app's connection id on the dataplane (often `1` when the uplink is `0`).
    #[clap(long, env)]
    #[arg(long, value_name = "CONN_INDEX")]
    dataplane_recv_conn: Option<u64>,
}

impl Args {
    pub fn config(&self) -> &String {
        &self.config
    }

    pub fn local_name(&self) -> &str {
        &self.local_name
    }

    pub fn remote_name(&self) -> &str {
        &self.remote_name
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn mls_group_id(&self) -> Option<&str> {
        self.mls_group_id.as_deref()
    }

    pub fn dataplane_forward_conn(&self) -> Option<u64> {
        self.dataplane_forward_conn
    }

    pub fn dataplane_recv_conn(&self) -> Option<u64> {
        self.dataplane_recv_conn
    }
}
