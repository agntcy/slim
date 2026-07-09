// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! `Display for ClientConfig` on native targets (full field set, including
//! auth/TLS/proxy/compression/backoff/metadata). The browser counterpart lives
//! in `client_display_wasm.rs`.

use crate::client::ClientConfig;

impl std::fmt::Display for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ClientConfig {{ endpoint: {}, transport: {:?}, origin: {:?}, server_name: {:?}, compression: {:?}, rate_limit: {:?}, tls_setting: {:?}, keepalive: {:?}, proxy: {:?}, connect_timeout: {:?}, request_timeout: {:?}, buffer_size: {:?}, headers: {:?}, auth: {:?}, backoff: {:?}, metadata: {:?}, link_id: {:?}, connection_type: {:?} }}",
            self.endpoint,
            self.resolved_transport(),
            self.origin,
            self.server_name,
            self.compression,
            self.rate_limit,
            self.tls_setting,
            self.keepalive,
            self.proxy,
            self.connect_timeout,
            self.request_timeout,
            self.buffer_size,
            self.headers,
            self.auth,
            self.backoff,
            self.metadata,
            self.link_id,
            self.connection_type
        )
    }
}
