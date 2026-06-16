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
//!
//! **Topology:** Only `FullMesh` is supported. `HubAndSpoke` requires static
//! discovery because hub election needs all peers to be known upfront.

use std::collections::HashMap;

use futures::TryStreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use kube::api::Api;
use kube::runtime::watcher;
use kube::runtime::watcher::Event as WatcherEvent;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use slim_config::client::ClientConfig;
use slim_config::conn_type::ConnType;

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
    pub fn new(namespace: String, label_selector: String, port: u16, self_node_id: String) -> Self {
        Self {
            namespace,
            label_selector,
            port,
            self_node_id,
            event_rx: None,
        }
    }

    /// Extract peer info from a pod, if it is ready and has an IP.
    fn peer_info_from_pod(pod: &Pod, port: u16, self_node_id: &str) -> Option<PeerInfo> {
        let name = pod.metadata.name.as_deref()?;
        let status = pod.status.as_ref()?;
        let pod_ip = status.pod_ip.as_deref()?;

        // Only consider pods that have at least one ready container condition.
        let is_ready = status
            .conditions
            .as_ref()
            .map(|conds| {
                conds
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            })
            .unwrap_or(false);

        if !is_ready {
            return None;
        }

        let endpoint = format!("http://{}:{}", pod_ip, port);
        let config = ClientConfig {
            endpoint,
            connection_type: ConnType::Peer,
            link_id: super::peer_link_id(self_node_id, name),
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

        let client = Client::try_default().await.map_err(|e| {
            PeerDiscoveryError::Backend(format!("failed to create k8s client: {e}"))
        })?;

        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);

        // Read own pod's pod-template-hash label so we only discover peers
        // from the same ReplicaSet (avoids cross-RS connections during rolling updates).
        let label_selector = {
            let self_pod = pods
                .get(&self.self_node_id)
                .await
                .map_err(|e| PeerDiscoveryError::Backend(format!("failed to get own pod: {e}")))?;
            let hash = self_pod
                .metadata
                .labels
                .as_ref()
                .and_then(|l| l.get("pod-template-hash"))
                .cloned();

            match hash {
                Some(h) => {
                    let selector = format!("{},pod-template-hash={}", self.label_selector, h);
                    info!(
                        selector = %selector,
                        "scoping k8s peer discovery to same ReplicaSet"
                    );
                    selector
                }
                None => {
                    warn!("pod-template-hash label not found, using base label selector");
                    self.label_selector.clone()
                }
            }
        };

        let (event_tx, event_rx) = mpsc::channel(64);
        self.event_rx = Some(event_rx);

        let self_node_id = self.self_node_id.clone();
        let port = self.port;

        // Spawn a background task that watches pods and produces PeerEvents.
        tokio::spawn(async move {
            let stream = watcher(pods, watcher::Config::default().labels(&label_selector));
            tokio::pin!(stream);

            // Track which pods we've emitted Joined for (pod_name → PeerInfo).
            let mut known_peers: HashMap<String, PeerInfo> = HashMap::new();
            // During re-list (Init phase), track which peers are still present.
            let mut init_seen: Option<HashMap<String, PeerInfo>> = None;

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

                                if let Some(info) =
                                    Self::peer_info_from_pod(&pod, port, &self_node_id)
                                {
                                    // During init phase, track seen peers.
                                    if let Some(ref mut seen) = init_seen {
                                        seen.insert(pod_name.clone(), info.clone());
                                    }

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
                                init_seen = Some(HashMap::new());
                            }
                            WatcherEvent::InitDone => {
                                // Reconcile: emit Left for peers that were known but
                                // not seen during this re-list.
                                if let Some(seen) = init_seen.take() {
                                    let stale: Vec<String> = known_peers
                                        .keys()
                                        .filter(|k| !seen.contains_key(*k))
                                        .cloned()
                                        .collect();

                                    for peer_name in stale {
                                        if let Some(info) = known_peers.remove(&peer_name) {
                                            info!(
                                                peer_id = %peer_name,
                                                "k8s peer gone after re-list"
                                            );
                                            if event_tx.send(PeerEvent::Left(info)).await.is_err() {
                                                return;
                                            }
                                        }
                                    }
                                }

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
            Some(rx) => rx.recv().await.ok_or(PeerDiscoveryError::Closed),
            None => Err(PeerDiscoveryError::Backend(
                "discovery not started".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{PodCondition, PodStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    /// Helper: build a Pod with the given name, IP, readiness, and optional labels.
    fn make_pod(
        name: &str,
        ip: Option<&str>,
        ready: bool,
        labels: Option<BTreeMap<String, String>>,
    ) -> Pod {
        let conditions = if ready {
            Some(vec![PodCondition {
                type_: "Ready".to_string(),
                status: "True".to_string(),
                ..Default::default()
            }])
        } else {
            Some(vec![PodCondition {
                type_: "Ready".to_string(),
                status: "False".to_string(),
                ..Default::default()
            }])
        };

        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                labels,
                ..Default::default()
            },
            status: Some(PodStatus {
                pod_ip: ip.map(|s| s.to_string()),
                conditions,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    // ── peer_info_from_pod ──────────────────────────────────────────────

    #[test]
    fn test_peer_info_ready_pod() {
        let pod = make_pod("slim-abc-12345", Some("10.0.0.1"), true, None);
        let info = KubernetesPeerDiscovery::peer_info_from_pod(&pod, 46357, "slim-abc-99999");
        let info = info.expect("ready pod should produce PeerInfo");

        assert_eq!(info.id, "slim-abc-12345");
        assert_eq!(info.config.endpoint, "http://10.0.0.1:46357");
        assert_eq!(info.config.connection_type, ConnType::Peer);
        assert_eq!(info.config.link_id, "slim-abc-99999:slim-abc-12345");
    }

    #[test]
    fn test_peer_info_not_ready_pod() {
        let pod = make_pod("slim-abc-12345", Some("10.0.0.1"), false, None);
        let info = KubernetesPeerDiscovery::peer_info_from_pod(&pod, 46357, "self");
        assert!(info.is_none(), "non-ready pod should return None");
    }

    #[test]
    fn test_peer_info_no_ip() {
        let pod = make_pod("slim-abc-12345", None, true, None);
        let info = KubernetesPeerDiscovery::peer_info_from_pod(&pod, 46357, "self");
        assert!(info.is_none(), "pod without IP should return None");
    }

    #[test]
    fn test_peer_info_no_name() {
        let mut pod = make_pod("temp", Some("10.0.0.1"), true, None);
        pod.metadata.name = None;
        let info = KubernetesPeerDiscovery::peer_info_from_pod(&pod, 46357, "self");
        assert!(info.is_none(), "pod without name should return None");
    }

    #[test]
    fn test_peer_info_no_status() {
        let mut pod = make_pod("slim-abc-12345", Some("10.0.0.1"), true, None);
        pod.status = None;
        let info = KubernetesPeerDiscovery::peer_info_from_pod(&pod, 46357, "self");
        assert!(info.is_none(), "pod without status should return None");
    }

    #[test]
    fn test_peer_info_no_conditions() {
        let mut pod = make_pod("slim-abc-12345", Some("10.0.0.1"), true, None);
        pod.status.as_mut().unwrap().conditions = None;
        let info = KubernetesPeerDiscovery::peer_info_from_pod(&pod, 46357, "self");
        assert!(info.is_none(), "pod without conditions should return None");
    }

    #[test]
    fn test_peer_info_different_port() {
        let pod = make_pod("peer-1", Some("192.168.1.5"), true, None);
        let info = KubernetesPeerDiscovery::peer_info_from_pod(&pod, 9090, "self").unwrap();
        assert_eq!(info.config.endpoint, "http://192.168.1.5:9090");
    }

    #[test]
    fn test_peer_info_link_id_is_directional() {
        let pod = make_pod("peer-b", Some("10.0.0.2"), true, None);

        let info_from_a =
            KubernetesPeerDiscovery::peer_info_from_pod(&pod, 46357, "peer-a").unwrap();
        assert_eq!(info_from_a.config.link_id, "peer-a:peer-b");

        let info_from_c =
            KubernetesPeerDiscovery::peer_info_from_pod(&pod, 46357, "peer-c").unwrap();
        assert_eq!(info_from_c.config.link_id, "peer-c:peer-b");
    }

    // ── constructor ─────────────────────────────────────────────────────

    #[test]
    fn test_new_initial_state() {
        let disc = KubernetesPeerDiscovery::new(
            "slim-ns".to_string(),
            "app=slim".to_string(),
            46357,
            "self-pod".to_string(),
        );
        assert_eq!(disc.namespace, "slim-ns");
        assert_eq!(disc.label_selector, "app=slim");
        assert_eq!(disc.port, 46357);
        assert_eq!(disc.self_node_id, "self-pod");
        assert!(disc.event_rx.is_none());
    }

    // ── recv before start ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_recv_before_start_returns_error() {
        let mut disc = KubernetesPeerDiscovery::new(
            "ns".to_string(),
            "app=slim".to_string(),
            46357,
            "self".to_string(),
        );
        let err = disc.recv().await;
        assert!(err.is_err());
        assert!(
            matches!(err, Err(PeerDiscoveryError::Backend(_))),
            "recv before start should return Backend error"
        );
    }

    // ── watcher event processing logic ──────────────────────────────────

    /// Simulates the watcher event processing loop to test the join/left/
    /// reconciliation logic without a real k8s cluster.
    async fn process_events(
        events: Vec<WatcherEvent<Pod>>,
        self_node_id: &str,
        port: u16,
    ) -> Vec<PeerEvent> {
        let (event_tx, mut event_rx) = mpsc::channel(64);
        let self_id = self_node_id.to_string();

        // Replicate the watcher processing logic from start().
        tokio::spawn(async move {
            let mut known_peers: HashMap<String, PeerInfo> = HashMap::new();
            let mut init_seen: Option<HashMap<String, PeerInfo>> = None;

            for event in events {
                match event {
                    WatcherEvent::Apply(pod) | WatcherEvent::InitApply(pod) => {
                        let pod_name = match pod.metadata.name.as_deref() {
                            Some(n) => n.to_string(),
                            None => continue,
                        };
                        if pod_name == self_id {
                            continue;
                        }
                        if let Some(info) =
                            KubernetesPeerDiscovery::peer_info_from_pod(&pod, port, &self_id)
                        {
                            if let Some(ref mut seen) = init_seen {
                                seen.insert(pod_name.clone(), info.clone());
                            }
                            if let std::collections::hash_map::Entry::Vacant(e) =
                                known_peers.entry(pod_name.clone())
                            {
                                e.insert(info.clone());
                                let _ = event_tx.send(PeerEvent::Joined(info)).await;
                            }
                        } else if let Some(info) = known_peers.remove(&pod_name) {
                            let _ = event_tx.send(PeerEvent::Left(info)).await;
                        }
                    }
                    WatcherEvent::Delete(pod) => {
                        let pod_name = match pod.metadata.name.as_deref() {
                            Some(n) => n.to_string(),
                            None => continue,
                        };
                        if let Some(info) = known_peers.remove(&pod_name) {
                            let _ = event_tx.send(PeerEvent::Left(info)).await;
                        }
                    }
                    WatcherEvent::Init => {
                        init_seen = Some(HashMap::new());
                    }
                    WatcherEvent::InitDone => {
                        if let Some(seen) = init_seen.take() {
                            let stale: Vec<String> = known_peers
                                .keys()
                                .filter(|k| !seen.contains_key(*k))
                                .cloned()
                                .collect();
                            for peer_name in stale {
                                if let Some(info) = known_peers.remove(&peer_name) {
                                    let _ = event_tx.send(PeerEvent::Left(info)).await;
                                }
                            }
                        }
                    }
                }
            }
            drop(event_tx);
        });

        let mut results = Vec::new();
        while let Some(ev) = event_rx.recv().await {
            results.push(ev);
        }
        results
    }

    #[tokio::test]
    async fn test_watcher_emits_joined_for_ready_pods() {
        let events = vec![
            WatcherEvent::Apply(make_pod("peer-a", Some("10.0.0.1"), true, None)),
            WatcherEvent::Apply(make_pod("peer-b", Some("10.0.0.2"), true, None)),
        ];
        let result = process_events(events, "self", 46357).await;

        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], PeerEvent::Joined(info) if info.id == "peer-a"));
        assert!(matches!(&result[1], PeerEvent::Joined(info) if info.id == "peer-b"));
    }

    #[tokio::test]
    async fn test_watcher_skips_self() {
        let events = vec![
            WatcherEvent::Apply(make_pod("self-pod", Some("10.0.0.1"), true, None)),
            WatcherEvent::Apply(make_pod("peer-a", Some("10.0.0.2"), true, None)),
        ];
        let result = process_events(events, "self-pod", 46357).await;

        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], PeerEvent::Joined(info) if info.id == "peer-a"));
    }

    #[tokio::test]
    async fn test_watcher_deduplicates_joined() {
        let events = vec![
            WatcherEvent::Apply(make_pod("peer-a", Some("10.0.0.1"), true, None)),
            WatcherEvent::Apply(make_pod("peer-a", Some("10.0.0.1"), true, None)),
        ];
        let result = process_events(events, "self", 46357).await;

        assert_eq!(
            result.len(),
            1,
            "duplicate Apply should not emit second Joined"
        );
    }

    #[tokio::test]
    async fn test_watcher_emits_left_when_not_ready() {
        let events = vec![
            WatcherEvent::Apply(make_pod("peer-a", Some("10.0.0.1"), true, None)),
            WatcherEvent::Apply(make_pod("peer-a", Some("10.0.0.1"), false, None)),
        ];
        let result = process_events(events, "self", 46357).await;

        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], PeerEvent::Joined(info) if info.id == "peer-a"));
        assert!(matches!(&result[1], PeerEvent::Left(info) if info.id == "peer-a"));
    }

    #[tokio::test]
    async fn test_watcher_emits_left_on_delete() {
        let events = vec![
            WatcherEvent::Apply(make_pod("peer-a", Some("10.0.0.1"), true, None)),
            WatcherEvent::Delete(make_pod("peer-a", Some("10.0.0.1"), true, None)),
        ];
        let result = process_events(events, "self", 46357).await;

        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], PeerEvent::Joined(info) if info.id == "peer-a"));
        assert!(matches!(&result[1], PeerEvent::Left(info) if info.id == "peer-a"));
    }

    #[tokio::test]
    async fn test_watcher_delete_unknown_pod_is_noop() {
        let events = vec![WatcherEvent::Delete(make_pod(
            "unknown",
            Some("10.0.0.1"),
            true,
            None,
        ))];
        let result = process_events(events, "self", 46357).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_watcher_reconcile_removes_stale_peers() {
        let events = vec![
            // Initial sync: peer-a and peer-b are discovered.
            WatcherEvent::Apply(make_pod("peer-a", Some("10.0.0.1"), true, None)),
            WatcherEvent::Apply(make_pod("peer-b", Some("10.0.0.2"), true, None)),
            // Re-list: only peer-a is present → peer-b is stale.
            WatcherEvent::Init,
            WatcherEvent::InitApply(make_pod("peer-a", Some("10.0.0.1"), true, None)),
            WatcherEvent::InitDone,
        ];
        let result = process_events(events, "self", 46357).await;

        assert_eq!(result.len(), 3);
        assert!(matches!(&result[0], PeerEvent::Joined(info) if info.id == "peer-a"));
        assert!(matches!(&result[1], PeerEvent::Joined(info) if info.id == "peer-b"));
        assert!(matches!(&result[2], PeerEvent::Left(info) if info.id == "peer-b"));
    }

    #[tokio::test]
    async fn test_watcher_reconcile_no_stale_peers() {
        let events = vec![
            WatcherEvent::Apply(make_pod("peer-a", Some("10.0.0.1"), true, None)),
            // Re-list: peer-a is still present.
            WatcherEvent::Init,
            WatcherEvent::InitApply(make_pod("peer-a", Some("10.0.0.1"), true, None)),
            WatcherEvent::InitDone,
        ];
        let result = process_events(events, "self", 46357).await;

        assert_eq!(result.len(), 1, "no stale peers, no extra Left events");
        assert!(matches!(&result[0], PeerEvent::Joined(info) if info.id == "peer-a"));
    }

    #[tokio::test]
    async fn test_watcher_reconcile_new_peer_during_relist() {
        let events = vec![
            WatcherEvent::Apply(make_pod("peer-a", Some("10.0.0.1"), true, None)),
            // Re-list: peer-a gone, peer-b is new.
            WatcherEvent::Init,
            WatcherEvent::InitApply(make_pod("peer-b", Some("10.0.0.2"), true, None)),
            WatcherEvent::InitDone,
        ];
        let result = process_events(events, "self", 46357).await;

        assert_eq!(result.len(), 3);
        assert!(matches!(&result[0], PeerEvent::Joined(info) if info.id == "peer-a"));
        assert!(matches!(&result[1], PeerEvent::Joined(info) if info.id == "peer-b"));
        assert!(matches!(&result[2], PeerEvent::Left(info) if info.id == "peer-a"));
    }

    #[tokio::test]
    async fn test_watcher_skips_not_ready_during_init() {
        let events = vec![
            WatcherEvent::Init,
            WatcherEvent::InitApply(make_pod("peer-a", Some("10.0.0.1"), false, None)),
            WatcherEvent::InitDone,
        ];
        let result = process_events(events, "self", 46357).await;
        assert!(
            result.is_empty(),
            "not-ready pods during init should not emit events"
        );
    }
}
