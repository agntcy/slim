// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Kubernetes-based peer discovery implementation.
//!
//! Watches [`EndpointSlice`] objects for a named Service and emits
//! [`PeerEvent::Joined`] / [`PeerEvent::Left`] events as endpoints become
//! ready or are removed.
//!
//! Each discovered endpoint is identified by the pod name from the
//! endpoint's `targetRef` (used as `node_id`), and the endpoint address
//! combined with the configured port forms the endpoint URL.
//!
//! **Topology:** Only `FullMesh` is supported. `HubAndSpoke` requires static
//! discovery because hub election needs all peers to be known upfront.

use std::collections::HashMap;

use futures::TryStreamExt;
use k8s_openapi::api::discovery::v1::EndpointSlice;
use kube::Client;
use kube::api::Api;
use kube::runtime::watcher;
use kube::runtime::watcher::Event as WatcherEvent;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use slim_config::client::ClientConfig;
use slim_config::conn_type::ConnType;

use super::{PeerDiscovery, PeerDiscoveryError, PeerEvent, PeerInfo};

/// Kubernetes peer discovery: watches EndpointSlices by service name.
///
/// Discovers peer SLIM replicas running in the same namespace by watching
/// the EndpointSlice objects associated with a headless or ClusterIP service.
/// Only ready endpoints are included (readiness is managed by Kubernetes).
pub struct KubernetesPeerDiscovery {
    /// Namespace to watch EndpointSlices in.
    namespace: String,
    /// Kubernetes Service name whose EndpointSlices to watch.
    service_name: String,
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
    /// - `service_name`: Kubernetes Service name to watch EndpointSlices for.
    /// - `port`: Port on which peer pods listen for dataplane connections.
    /// - `self_node_id`: This node's pod name (will be excluded from events).
    pub fn new(namespace: String, service_name: String, port: u16, self_node_id: String) -> Self {
        Self {
            namespace,
            service_name,
            port,
            self_node_id,
            event_rx: None,
        }
    }

    /// Extract ready peers from an EndpointSlice.
    ///
    /// Returns a map of pod_name → PeerInfo for all ready endpoints
    /// that have a pod targetRef and at least one address.
    fn peers_from_slice(
        slice: &EndpointSlice,
        port: u16,
        self_node_id: &str,
    ) -> HashMap<String, PeerInfo> {
        let mut peers = HashMap::new();

        for ep in &slice.endpoints {
            // Only include ready endpoints (default to ready if unset).
            let is_ready = ep.conditions.as_ref().and_then(|c| c.ready).unwrap_or(true);
            if !is_ready {
                continue;
            }

            // Get pod name from targetRef.
            let pod_name = match ep.target_ref.as_ref() {
                Some(r) if r.kind.as_deref() == Some("Pod") => match r.name.as_deref() {
                    Some(n) => n,
                    None => continue,
                },
                _ => continue,
            };

            // Skip self.
            if pod_name == self_node_id {
                continue;
            }

            // Use the first address.
            let addr = match ep.addresses.first() {
                Some(a) => a.as_str(),
                None => continue,
            };

            let endpoint_url = format!("http://{}:{}", addr, port);
            let config = ClientConfig {
                endpoint: endpoint_url,
                connection_type: ConnType::Peer,
                link_id: super::peer_link_id(self_node_id, pod_name),
                ..Default::default()
            };

            peers.insert(
                pod_name.to_string(),
                PeerInfo {
                    id: pod_name.to_string(),
                    config,
                },
            );
        }

        peers
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

        let slices: Api<EndpointSlice> = Api::namespaced(client, &self.namespace);
        let label_selector = format!("kubernetes.io/service-name={}", self.service_name);

        info!(
            service = %self.service_name,
            namespace = %self.namespace,
            selector = %label_selector,
            "starting k8s EndpointSlice watcher"
        );

        let (event_tx, event_rx) = mpsc::channel(64);
        self.event_rx = Some(event_rx);

        let self_node_id = self.self_node_id.clone();
        let port = self.port;

        // Spawn a background task that watches EndpointSlices and produces PeerEvents.
        tokio::spawn(async move {
            let stream = watcher(slices, watcher::Config::default().labels(&label_selector));
            tokio::pin!(stream);

            // Per-slice state: slice_name → (pod_name → PeerInfo).
            let mut slices_state: HashMap<String, HashMap<String, PeerInfo>> = HashMap::new();
            // Aggregate of all known peers across all slices: pod_name → PeerInfo.
            let mut known_peers: HashMap<String, PeerInfo> = HashMap::new();
            // During re-list (Init phase), track which slices are still present.
            let mut init_slices: Option<HashMap<String, HashMap<String, PeerInfo>>> = None;

            loop {
                match stream.try_next().await {
                    Ok(Some(event)) => {
                        match event {
                            WatcherEvent::Apply(slice) | WatcherEvent::InitApply(slice) => {
                                let slice_name = match slice.metadata.name.as_deref() {
                                    Some(n) => n.to_string(),
                                    None => continue,
                                };

                                let new_peers = Self::peers_from_slice(&slice, port, &self_node_id);

                                // During init phase, track seen slices.
                                if let Some(ref mut seen) = init_slices {
                                    seen.insert(slice_name.clone(), new_peers.clone());
                                }

                                let old_peers = slices_state
                                    .insert(slice_name.clone(), new_peers.clone())
                                    .unwrap_or_default();

                                // Emit Left for peers that were in this slice but are gone.
                                for (pod_name, info) in &old_peers {
                                    if !new_peers.contains_key(pod_name) {
                                        // Only emit Left if no other slice still has this peer.
                                        let in_other_slice = slices_state.iter().any(|(sn, ps)| {
                                            sn != &slice_name && ps.contains_key(pod_name)
                                        });
                                        if !in_other_slice && known_peers.remove(pod_name).is_some()
                                        {
                                            info!(
                                                peer_id = %pod_name,
                                                "k8s peer removed from EndpointSlice"
                                            );
                                            if event_tx
                                                .send(PeerEvent::Left(info.clone()))
                                                .await
                                                .is_err()
                                            {
                                                return;
                                            }
                                        }
                                    }
                                }

                                // Emit Joined for new peers.
                                for (pod_name, info) in &new_peers {
                                    if !known_peers.contains_key(pod_name) {
                                        info!(
                                            peer_id = %pod_name,
                                            endpoint = %info.config.endpoint,
                                            "k8s peer discovered via EndpointSlice"
                                        );
                                        known_peers.insert(pod_name.clone(), info.clone());
                                        if event_tx
                                            .send(PeerEvent::Joined(info.clone()))
                                            .await
                                            .is_err()
                                        {
                                            return;
                                        }
                                    }
                                }
                            }
                            WatcherEvent::Delete(slice) => {
                                let slice_name = match slice.metadata.name.as_deref() {
                                    Some(n) => n.to_string(),
                                    None => continue,
                                };

                                if let Some(old_peers) = slices_state.remove(&slice_name) {
                                    for (pod_name, info) in old_peers {
                                        // Only emit Left if no other slice still has this peer.
                                        let in_other_slice = slices_state
                                            .values()
                                            .any(|ps| ps.contains_key(&pod_name));
                                        if !in_other_slice
                                            && known_peers.remove(&pod_name).is_some()
                                        {
                                            info!(
                                                peer_id = %pod_name,
                                                "k8s peer gone (EndpointSlice deleted)"
                                            );
                                            if event_tx.send(PeerEvent::Left(info)).await.is_err() {
                                                return;
                                            }
                                        }
                                    }
                                }
                            }
                            WatcherEvent::Init => {
                                debug!("k8s EndpointSlice watcher initializing");
                                init_slices = Some(HashMap::new());
                            }
                            WatcherEvent::InitDone => {
                                // Reconcile: emit Left for peers from slices
                                // that were not seen during re-list.
                                if let Some(seen) = init_slices.take() {
                                    let stale_slices: Vec<String> = slices_state
                                        .keys()
                                        .filter(|k| !seen.contains_key(*k))
                                        .cloned()
                                        .collect();

                                    for slice_name in stale_slices {
                                        if let Some(old_peers) = slices_state.remove(&slice_name) {
                                            for (pod_name, info) in old_peers {
                                                let in_other_slice = slices_state
                                                    .values()
                                                    .any(|ps| ps.contains_key(&pod_name));
                                                if !in_other_slice
                                                    && known_peers.remove(&pod_name).is_some()
                                                {
                                                    info!(
                                                        peer_id = %pod_name,
                                                        "k8s peer gone after re-list"
                                                    );
                                                    if event_tx
                                                        .send(PeerEvent::Left(info))
                                                        .await
                                                        .is_err()
                                                    {
                                                        return;
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // Update slices_state with the new view.
                                    slices_state = seen;
                                }

                                info!(
                                    num_peers = known_peers.len(),
                                    "k8s EndpointSlice watcher initial sync complete"
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        warn!("k8s EndpointSlice watcher stream ended");
                        return;
                    }
                    Err(e) => {
                        warn!(error = %e, "k8s EndpointSlice watcher error, will retry");
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
    use k8s_openapi::api::core::v1::ObjectReference;
    use k8s_openapi::api::discovery::v1::{Endpoint, EndpointConditions};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    /// Helper: build an Endpoint with target pod ref, address, and readiness.
    fn make_endpoint(pod_name: &str, addr: &str, ready: bool) -> Endpoint {
        Endpoint {
            addresses: vec![addr.to_string()],
            conditions: Some(EndpointConditions {
                ready: Some(ready),
                serving: None,
                terminating: None,
            }),
            target_ref: Some(ObjectReference {
                kind: Some("Pod".to_string()),
                name: Some(pod_name.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Helper: build an EndpointSlice with the given name and endpoints.
    fn make_slice(name: &str, endpoints: Vec<Endpoint>) -> EndpointSlice {
        EndpointSlice {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            },
            address_type: "IPv4".to_string(),
            endpoints,
            ports: None,
        }
    }

    // ── peers_from_slice ────────────────────────────────────────────────

    #[test]
    fn test_peers_from_slice_ready_endpoints() {
        let slice = make_slice(
            "svc-abc",
            vec![
                make_endpoint("peer-a", "10.0.0.1", true),
                make_endpoint("peer-b", "10.0.0.2", true),
            ],
        );
        let peers = KubernetesPeerDiscovery::peers_from_slice(&slice, 46357, "self-pod");
        assert_eq!(peers.len(), 2);
        assert_eq!(peers["peer-a"].config.endpoint, "http://10.0.0.1:46357");
        assert_eq!(peers["peer-b"].config.endpoint, "http://10.0.0.2:46357");
        assert_eq!(peers["peer-a"].config.connection_type, ConnType::Peer);
    }

    #[test]
    fn test_peers_from_slice_skips_not_ready() {
        let slice = make_slice(
            "svc-abc",
            vec![
                make_endpoint("peer-a", "10.0.0.1", true),
                make_endpoint("peer-b", "10.0.0.2", false),
            ],
        );
        let peers = KubernetesPeerDiscovery::peers_from_slice(&slice, 46357, "self-pod");
        assert_eq!(peers.len(), 1);
        assert!(peers.contains_key("peer-a"));
    }

    #[test]
    fn test_peers_from_slice_skips_self() {
        let slice = make_slice(
            "svc-abc",
            vec![
                make_endpoint("self-pod", "10.0.0.1", true),
                make_endpoint("peer-a", "10.0.0.2", true),
            ],
        );
        let peers = KubernetesPeerDiscovery::peers_from_slice(&slice, 46357, "self-pod");
        assert_eq!(peers.len(), 1);
        assert!(peers.contains_key("peer-a"));
    }

    #[test]
    fn test_peers_from_slice_skips_no_target_ref() {
        let mut ep = make_endpoint("peer-a", "10.0.0.1", true);
        ep.target_ref = None;
        let slice = make_slice("svc-abc", vec![ep]);
        let peers = KubernetesPeerDiscovery::peers_from_slice(&slice, 46357, "self-pod");
        assert!(peers.is_empty());
    }

    #[test]
    fn test_peers_from_slice_skips_no_address() {
        let mut ep = make_endpoint("peer-a", "10.0.0.1", true);
        ep.addresses.clear();
        let slice = make_slice("svc-abc", vec![ep]);
        let peers = KubernetesPeerDiscovery::peers_from_slice(&slice, 46357, "self-pod");
        assert!(peers.is_empty());
    }

    #[test]
    fn test_peers_from_slice_link_id_is_directional() {
        let slice = make_slice("svc-abc", vec![make_endpoint("peer-b", "10.0.0.2", true)]);
        let peers = KubernetesPeerDiscovery::peers_from_slice(&slice, 46357, "peer-a");
        assert_eq!(peers["peer-b"].config.link_id, "peer-a:peer-b");
    }

    #[test]
    fn test_peers_from_slice_ready_defaults_true_when_unset() {
        let mut ep = make_endpoint("peer-a", "10.0.0.1", true);
        ep.conditions = None; // unset → defaults to ready
        let slice = make_slice("svc-abc", vec![ep]);
        let peers = KubernetesPeerDiscovery::peers_from_slice(&slice, 46357, "self-pod");
        assert_eq!(peers.len(), 1);
    }

    // ── constructor ─────────────────────────────────────────────────────

    #[test]
    fn test_new_initial_state() {
        let disc = KubernetesPeerDiscovery::new(
            "slim-ns".to_string(),
            "slim-svc".to_string(),
            46357,
            "self-pod".to_string(),
        );
        assert_eq!(disc.namespace, "slim-ns");
        assert_eq!(disc.service_name, "slim-svc");
        assert_eq!(disc.port, 46357);
        assert_eq!(disc.self_node_id, "self-pod");
        assert!(disc.event_rx.is_none());
    }

    // ── recv before start ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_recv_before_start_returns_error() {
        let mut disc = KubernetesPeerDiscovery::new(
            "ns".to_string(),
            "svc".to_string(),
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
        events: Vec<WatcherEvent<EndpointSlice>>,
        self_node_id: &str,
        port: u16,
    ) -> Vec<PeerEvent> {
        let (event_tx, mut event_rx) = mpsc::channel(64);
        let self_id = self_node_id.to_string();

        tokio::spawn(async move {
            let mut slices_state: HashMap<String, HashMap<String, PeerInfo>> = HashMap::new();
            let mut known_peers: HashMap<String, PeerInfo> = HashMap::new();
            let mut init_slices: Option<HashMap<String, HashMap<String, PeerInfo>>> = None;

            for event in events {
                match event {
                    WatcherEvent::Apply(slice) | WatcherEvent::InitApply(slice) => {
                        let slice_name = match slice.metadata.name.as_deref() {
                            Some(n) => n.to_string(),
                            None => continue,
                        };

                        let new_peers =
                            KubernetesPeerDiscovery::peers_from_slice(&slice, port, &self_id);

                        if let Some(ref mut seen) = init_slices {
                            seen.insert(slice_name.clone(), new_peers.clone());
                        }

                        let old_peers = slices_state
                            .insert(slice_name.clone(), new_peers.clone())
                            .unwrap_or_default();

                        // Left for removed peers.
                        for (pod_name, info) in &old_peers {
                            if !new_peers.contains_key(pod_name) {
                                let in_other = slices_state
                                    .iter()
                                    .any(|(sn, ps)| sn != &slice_name && ps.contains_key(pod_name));
                                if !in_other && known_peers.remove(pod_name).is_some() {
                                    let _ = event_tx.send(PeerEvent::Left(info.clone())).await;
                                }
                            }
                        }

                        // Joined for new peers.
                        for (pod_name, info) in &new_peers {
                            if !known_peers.contains_key(pod_name) {
                                known_peers.insert(pod_name.clone(), info.clone());
                                let _ = event_tx.send(PeerEvent::Joined(info.clone())).await;
                            }
                        }
                    }
                    WatcherEvent::Delete(slice) => {
                        let slice_name = match slice.metadata.name.as_deref() {
                            Some(n) => n.to_string(),
                            None => continue,
                        };
                        if let Some(old_peers) = slices_state.remove(&slice_name) {
                            for (pod_name, info) in old_peers {
                                let in_other =
                                    slices_state.values().any(|ps| ps.contains_key(&pod_name));
                                if !in_other && known_peers.remove(&pod_name).is_some() {
                                    let _ = event_tx.send(PeerEvent::Left(info)).await;
                                }
                            }
                        }
                    }
                    WatcherEvent::Init => {
                        init_slices = Some(HashMap::new());
                    }
                    WatcherEvent::InitDone => {
                        if let Some(seen) = init_slices.take() {
                            let stale: Vec<String> = slices_state
                                .keys()
                                .filter(|k| !seen.contains_key(*k))
                                .cloned()
                                .collect();
                            for slice_name in stale {
                                if let Some(old_peers) = slices_state.remove(&slice_name) {
                                    for (pod_name, info) in old_peers {
                                        let in_other = slices_state
                                            .values()
                                            .any(|ps| ps.contains_key(&pod_name));
                                        if !in_other && known_peers.remove(&pod_name).is_some() {
                                            let _ = event_tx.send(PeerEvent::Left(info)).await;
                                        }
                                    }
                                }
                            }
                            slices_state = seen;
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
    async fn test_watcher_emits_joined_for_ready_endpoints() {
        let events = vec![WatcherEvent::Apply(make_slice(
            "svc-abc",
            vec![
                make_endpoint("peer-a", "10.0.0.1", true),
                make_endpoint("peer-b", "10.0.0.2", true),
            ],
        ))];
        let result = process_events(events, "self", 46357).await;

        assert_eq!(result.len(), 2);
        let ids: Vec<&str> = result
            .iter()
            .filter_map(|e| match e {
                PeerEvent::Joined(info) => Some(info.id.as_str()),
                _ => None,
            })
            .collect();
        assert!(ids.contains(&"peer-a"));
        assert!(ids.contains(&"peer-b"));
    }

    #[tokio::test]
    async fn test_watcher_skips_self() {
        let events = vec![WatcherEvent::Apply(make_slice(
            "svc-abc",
            vec![
                make_endpoint("self-pod", "10.0.0.1", true),
                make_endpoint("peer-a", "10.0.0.2", true),
            ],
        ))];
        let result = process_events(events, "self-pod", 46357).await;

        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], PeerEvent::Joined(info) if info.id == "peer-a"));
    }

    #[tokio::test]
    async fn test_watcher_deduplicates_joined() {
        let events = vec![
            WatcherEvent::Apply(make_slice(
                "svc-abc",
                vec![make_endpoint("peer-a", "10.0.0.1", true)],
            )),
            WatcherEvent::Apply(make_slice(
                "svc-abc",
                vec![make_endpoint("peer-a", "10.0.0.1", true)],
            )),
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
            WatcherEvent::Apply(make_slice(
                "svc-abc",
                vec![make_endpoint("peer-a", "10.0.0.1", true)],
            )),
            WatcherEvent::Apply(make_slice(
                "svc-abc",
                vec![make_endpoint("peer-a", "10.0.0.1", false)],
            )),
        ];
        let result = process_events(events, "self", 46357).await;

        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], PeerEvent::Joined(info) if info.id == "peer-a"));
        assert!(matches!(&result[1], PeerEvent::Left(info) if info.id == "peer-a"));
    }

    #[tokio::test]
    async fn test_watcher_emits_left_on_delete() {
        let events = vec![
            WatcherEvent::Apply(make_slice(
                "svc-abc",
                vec![make_endpoint("peer-a", "10.0.0.1", true)],
            )),
            WatcherEvent::Delete(make_slice(
                "svc-abc",
                vec![make_endpoint("peer-a", "10.0.0.1", true)],
            )),
        ];
        let result = process_events(events, "self", 46357).await;

        assert_eq!(result.len(), 2);
        assert!(matches!(&result[0], PeerEvent::Joined(info) if info.id == "peer-a"));
        assert!(matches!(&result[1], PeerEvent::Left(info) if info.id == "peer-a"));
    }

    #[tokio::test]
    async fn test_watcher_delete_unknown_slice_is_noop() {
        let events = vec![WatcherEvent::Delete(make_slice(
            "unknown",
            vec![make_endpoint("peer-a", "10.0.0.1", true)],
        ))];
        let result = process_events(events, "self", 46357).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_watcher_reconcile_removes_stale_peers() {
        let events = vec![
            // Initial sync: slice with peer-a and peer-b.
            WatcherEvent::Apply(make_slice(
                "svc-abc",
                vec![
                    make_endpoint("peer-a", "10.0.0.1", true),
                    make_endpoint("peer-b", "10.0.0.2", true),
                ],
            )),
            // Re-list: slice now only has peer-a → peer-b is stale.
            WatcherEvent::Init,
            WatcherEvent::InitApply(make_slice(
                "svc-abc",
                vec![make_endpoint("peer-a", "10.0.0.1", true)],
            )),
            WatcherEvent::InitDone,
        ];
        let result = process_events(events, "self", 46357).await;

        assert_eq!(result.len(), 3);
        let ids: Vec<(&str, bool)> = result
            .iter()
            .map(|e| match e {
                PeerEvent::Joined(info) => (info.id.as_str(), true),
                PeerEvent::Left(info) => (info.id.as_str(), false),
            })
            .collect();
        // peer-a and peer-b joined, then peer-b left
        assert!(ids.contains(&("peer-a", true)));
        assert!(ids.contains(&("peer-b", true)));
        assert!(ids.contains(&("peer-b", false)));
    }

    #[tokio::test]
    async fn test_watcher_reconcile_stale_slice() {
        let events = vec![
            // Initial sync: two slices.
            WatcherEvent::Apply(make_slice(
                "svc-abc",
                vec![make_endpoint("peer-a", "10.0.0.1", true)],
            )),
            WatcherEvent::Apply(make_slice(
                "svc-def",
                vec![make_endpoint("peer-b", "10.0.0.2", true)],
            )),
            // Re-list: only svc-abc is present → svc-def (peer-b) is stale.
            WatcherEvent::Init,
            WatcherEvent::InitApply(make_slice(
                "svc-abc",
                vec![make_endpoint("peer-a", "10.0.0.1", true)],
            )),
            WatcherEvent::InitDone,
        ];
        let result = process_events(events, "self", 46357).await;

        assert_eq!(result.len(), 3);
        assert!(matches!(&result[2], PeerEvent::Left(info) if info.id == "peer-b"));
    }

    #[tokio::test]
    async fn test_watcher_peer_in_multiple_slices_not_removed_until_all_gone() {
        let events = vec![
            // peer-a appears in two slices.
            WatcherEvent::Apply(make_slice(
                "svc-abc",
                vec![make_endpoint("peer-a", "10.0.0.1", true)],
            )),
            WatcherEvent::Apply(make_slice(
                "svc-def",
                vec![make_endpoint("peer-a", "10.0.0.1", true)],
            )),
            // Remove from first slice only.
            WatcherEvent::Apply(make_slice("svc-abc", vec![])),
        ];
        let result = process_events(events, "self", 46357).await;

        // Should only get one Joined, no Left (peer-a still in svc-def).
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], PeerEvent::Joined(info) if info.id == "peer-a"));
    }

    #[tokio::test]
    async fn test_watcher_skips_not_ready_during_init() {
        let events = vec![
            WatcherEvent::Init,
            WatcherEvent::InitApply(make_slice(
                "svc-abc",
                vec![make_endpoint("peer-a", "10.0.0.1", false)],
            )),
            WatcherEvent::InitDone,
        ];
        let result = process_events(events, "self", 46357).await;
        assert!(
            result.is_empty(),
            "not-ready endpoints during init should not emit events"
        );
    }
}
