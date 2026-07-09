// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! `Display for ClientConfig` in the browser build. The browser has no
//! auth/TLS/proxy/compression fields, so they are omitted here. The native
//! counterpart lives in `client_display_native.rs`.

use crate::client::ClientConfig;

impl std::fmt::Display for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ClientConfig {{ endpoint: {}, transport: {:?}, origin: {:?}, server_name: {:?}, rate_limit: {:?}, keepalive: {:?}, connect_timeout: {:?}, request_timeout: {:?}, buffer_size: {:?}, headers: {:?}, link_id: {:?}, connection_type: {:?} }}",
            self.endpoint,
            self.resolved_transport(),
            self.origin,
            self.server_name,
            self.rate_limit,
            self.keepalive,
            self.connect_timeout,
            self.request_timeout,
            self.buffer_size,
            self.headers,
            self.link_id,
            self.connection_type
        )
    }
}
