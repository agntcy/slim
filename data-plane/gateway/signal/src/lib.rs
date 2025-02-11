// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

pub async fn shutdown() {
    imp::shutdown().await
}

#[cfg(unix)]
mod imp {
    use tokio::signal::unix::{signal, SignalKind};
    use tracing::info;

    pub(super) async fn shutdown() {
        tokio::select! {
            // this will handle interrupt signal by users
            _ = sig(SignalKind::interrupt(), "SIGINT") => {}
            // this will handle SIGTERM signal
            // e.g. k8s send this signal to stop the container
            _ = sig(SignalKind::terminate(), "SIGTERM") => {}
        };
    }

    async fn sig(kind: SignalKind, name: &str) {
        signal(kind)
            .expect("Failed to register signal handler")
            .recv()
            .await;
        info!(
            target: "gateway::signal",
            "received signal {}, starting shutdown",
            name,
        );
    }
}

#[cfg(not(unix))]
mod imp {
    use tracing::info;

    pub(super) async fn shutdown() {
        tokio::signal::windows::ctrl_c()
            .expect("Failed to register signal handler")
            .recv()
            .await;
        info!(
            target: "gateway::signal",
            "received signal Ctrl-C, starting shutdown",
        );
    }
}
