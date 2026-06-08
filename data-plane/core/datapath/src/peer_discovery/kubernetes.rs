// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Kubernetes-based peer discovery implementation.
//!
//! Watches pods matching a label selector in a given namespace and emits
//! [`PeerEvent::Joined`] / [`PeerEvent::Left`] events as pods become ready
//! or are removed.
//!
//! Each discovered pod is identified by its pod name (used as `node_id`) and
//! its pod IP combined with the configured port forms the endpoint URL.

use std::collections::HashMap;

use futures::TryStreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::Api;
use kube::runtime::watcher;
use kube::runtime::watcher::Event as WatcherEvent;
use kube::Client;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use slim_config::client::{ClientConfig, ConnType};

use super::{PeerDiscovery, PeerDiscoveryError, PeerEvent, PeerInfo};

/// Kubernetes peer discovery: watches pods by label selector.
///
/// Discovers peer SLIM replicas running in the same namespace and emits
/// events as their readiness state changes.
pub struct KubernetesPeerDiscovery {
    /// Namespace to watch pods in.
    namespace: String,
    /// Label selector for filtering peer pods.
    label_selector: String,
    /// Port on which peer pods listen for dataplane connections.
    port: u16,
    /// This node's own pod name (excluded from discovered peers).
    self_node_id: String,
    /// Receiver for events produced by the background watcher task.
    event_rx: Option<mpsc::Receiver<PeerEvent>>,
}

impl KubernetesPeerDiscovery {
    /// Create a new Kubernetes peer discovery instance.
    ///
    /// - `namespace`: Kubernetes namespace to watch.
    /// - `label_selector`: Label selector to filter peer pods (e.g., "app=slim").
    /// - `port`: Port on which peer pods listen for dataplane connections.
    /// - `self_node_id`: This node's pod name (will be excluded from events).
    pub fn new(
        namespace: String,
        label_selector: String,
        port: u16,
        self_node_id: String,
    ) -> Self {
        Self {
            namespace,
            label_selector,
            port,
            self_node_id,
            event_rx: None,
        }
    }

    /// Extract peer info from a pod, if it is ready and has an IP.
    fn peer_info_from_pod(pod: &Pod, port: u16) -> Option<PeerInfo> {
        let name = pod.metadata.name.as_deref()?;
        let status = pod.status.as_ref()?;
        let pod_ip = status.pod_ip.as_deref()?;

        // Only consider pods that have at least one ready container condition.
        let is_ready = status
            .conditions
            .as_ref()
            .map(|conds| {
                conds.iter().any(|c| {
                    c.type_ == "Ready" && c.status == "True"
                })
            })
            .unwrap_or(false);

        if !is_ready {
            return None;
        }

        let endpoint = format!("http://{}:{}", pod_ip, port);
        let config = ClientConfig {
            endpoint,
            connection_type: ConnType::Peer,
            ..Default::default()
        };

        Some(PeerInfo {
            id: name.to_string(),
            config,
        })
    }
}

impl PeerDiscovery for KubernetesPeerDiscovery {
    async fn start(&mut self) -> Result<(), PeerDiscoveryError> {
        if self.event_rx.is_some() {
            return Ok(());
        }

        let client = Client::try_default()
            .await
            .map_err(|e| PeerDiscoveryError::Backend(format!("failed to create k8s client: {e}")))?;

        let pods: Api<Pod> = Api::namespaced(client, &self.namespace);

        let (event_tx, event_rx) = mpsc::channel(64);
        self.event_rx = Some(event_rx);

        let self_node_id = self.self_node_id.clone();
        let label_selector = self.label_selector.clone();
        let port = self.port;

        // Spawn a background task that watches pods and produces PeerEvents.
        tokio::spawn(async move {
            let stream = watcher(pods, watcher::Config::default().labels(&label_selector));
            tokio::pin!(stream);

            // Track which pods we've emitted Joined for (pod_name → PeerInfo).
            let mut known_peers: HashMap<String, PeerInfo> = HashMap::new();

            loop {
                match stream.try_next().await {
                    Ok(Some(event)) => {
                        match event {
                            WatcherEvent::Apply(pod) | WatcherEvent::InitApply(pod) => {
                                let pod_name = match pod.metadata.name.as_deref() {
                                    Some(n) => n.to_string(),
                                    None => continue,
                                };

                                // Skip self.
                                if pod_name == self_node_id {
                                    continue;
                                }

                                if let Some(info) = Self::peer_info_from_pod(&pod, port) {
                                    if let std::collections::hash_map::Entry::Vacant(e) =
                                        known_peers.entry(pod_name.clone())
                                    {
                                        info!(
                                            peer_id = %pod_name,
                                            endpoint = %info.config.endpoint,
                                            "k8s peer discovered"
                                        );
                                        e.insert(info.clone());
                                        if event_tx.send(PeerEvent::Joined(info)).await.is_err() {
                                            return; // receiver dropped
                                        }
                                    }
                                } else {
                                    // Pod no longer ready — treat as Left.
                                    if let Some(info) = known_peers.remove(&pod_name) {
                                        info!(
                                            peer_id = %pod_name,
                                            "k8s peer no longer ready"
                                        );
                                        if event_tx.send(PeerEvent::Left(info)).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                            }
                            WatcherEvent::Delete(pod) => {
                                let pod_name = match pod.metadata.name.as_deref() {
                                    Some(n) => n.to_string(),
                                    None => continue,
                                };

                                if let Some(info) = known_peers.remove(&pod_name) {
                                    info!(
                                        peer_id = %pod_name,
                                        "k8s peer deleted"
                                    );
                                    if event_tx.send(PeerEvent::Left(info)).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            WatcherEvent::Init => {
                                debug!("k8s watcher initializing");
                            }
                            WatcherEvent::InitDone => {
                                info!(
                                    num_peers = known_peers.len(),
                                    "k8s watcher initial sync complete"
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        warn!("k8s watcher stream ended");
                        return;
                    }
                    Err(e) => {
                        warn!(error = %e, "k8s watcher error, will retry");
                        // kube-runtime watcher handles retries internally,
                        // but if the stream terminates we exit.
                    }
                }
            }
        });

        Ok(())
    }

    async fn recv(&mut self) -> Result<PeerEvent, PeerDiscoveryError> {
        match &mut self.event_rx {
            Some(rx) => rx
                .recv()
                .await
                .ok_or(PeerDiscoveryError::Closed),
            None => Err(PeerDiscoveryError::Backend(
                "discovery not started".to_string(),
            )),
        }
    }
}
