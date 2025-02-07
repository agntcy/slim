// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

#![deny(rust_2018_idioms, clippy::disallowed_methods, clippy::disallowed_types)]
#![forbid(unsafe_code)]

/// Returns a `Future` that completes when the gateway should start to shutdown.
pub async fn shutdown() {
    imp::shutdown().await
}

#[cfg(unix)]
mod imp {
    use tokio::signal::unix::{signal, SignalKind};
    use tracing::info;

    pub(super) async fn shutdown() {
        tokio::select! {
            // SIGINT  - To allow Ctrl-c to emulate SIGTERM while developing.
            () = sig(SignalKind::interrupt(), "SIGINT") => {}
            // SIGTERM - Kubernetes sends this to start a graceful shutdown.
            () = sig(SignalKind::terminate(), "SIGTERM") => {}
        };
    }

    async fn sig(kind: SignalKind, name: &'static str) {
        // Create a Future that completes the first
        // time the process receives 'sig'.
        signal(kind)
            .expect("Failed to register signal handler")
            .recv()
            .await;
        info!(
            // use target to remove 'imp' from output
            target: "gateway::signal",
            "received {}, starting shutdown",
            name,
        );
    }
}

#[cfg(not(unix))]
mod imp {
    use tracing::info;

    pub(super) async fn shutdown() {
        // On Windows, we don't have all the signals. This implementation allows
        // developers on Windows to simulate gateway graceful shutdown
        // by pressing Ctrl-C.
        tokio::signal::windows::ctrl_c()
            .expect("Failed to register signal handler")
            .recv()
            .await;
        info!(
            // use target to remove 'imp' from output
            target: "gateway::signal",
            "received Ctrl-C, starting shutdown",
        );
    }
}
