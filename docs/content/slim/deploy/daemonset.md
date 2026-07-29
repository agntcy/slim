# Deployment: Kubernetes DaemonSet

The DaemonSet deployment runs one SLIM pod on every node in the Kubernetes cluster. It is designed for workloads where application pods should always route messages through a SLIM pod on the same node, minimising latency and avoiding cross-node traffic for local communication.

## When to Use DaemonSet

Use the DaemonSet pattern when:

- **Low-latency routing matters** — traffic between a pod and its local SLIM node stays in-node memory without crossing the network
- **Node affinity is required** — each application pod must always use the SLIM instance on its own node (similar to a sidecar but managed centrally)
- **You want predictable scaling** — the number of SLIM pods is always equal to the number of nodes, without managing replica counts

For general-purpose multi-replica deployments, use [Kubernetes: Single Cluster](./kubernetes.md) instead.

## How It Works

The Helm chart creates a Kubernetes `DaemonSet` instead of a `Deployment`, with the data plane `Service` configured with `internalTrafficPolicy: Local`. This means any pod sending traffic to the `ClusterIP` service address is automatically routed to the SLIM pod on the same node — no special application configuration is needed.

```
Node A                          Node B
┌─────────────────────┐         ┌─────────────────────┐
│  App Pod            │         │  App Pod             │
│     ↓ (ClusterIP)   │         │     ↓ (ClusterIP)    │
│  SLIM Pod (local)   │ ──────▶ │  SLIM Pod (local)    │
└─────────────────────┘         └─────────────────────┘
        internalTrafficPolicy: Local on the Service
```

## Deployment

**`daemonset-values.yaml`:**

```yaml
slim:
  deploymentMode: DaemonSet

  image:
    repository: ghcr.io/agntcy/slim
    pullPolicy: IfNotPresent
    tag: ""

  config:
    tracing:
      log_level: info
    runtime:
      n_cores: 0
      thread_name: "slim-data-plane"
      drain_timeout: 10s
    services:
      slim/0:
        node_id: ${env:SLIM_SVC_ID}
        dataplane:
          servers:
            - endpoint: "0.0.0.0:46357"
              tls:
                insecure: true
          clients: []
        controller:
          clients:
            - endpoint: "https://slim-control:50052"
              tls:
                insecure: true

  # DaemonSet-specific settings
  hostNetwork: false
  dnsPolicy: "ClusterFirst"

  updateStrategy:
    type: "RollingUpdate"
    rollingUpdate:
      maxUnavailable: 1

  service:
    type: ClusterIP
    data:
      - port: 46357
        name: data-plane-0
    control:
      - port: 46358
```

Deploy:

```bash
helm upgrade --install slim \
  oci://ghcr.io/agntcy/slim/helm/slim \
  --namespace slim \
  --create-namespace \
  --values daemonset-values.yaml
```

Verify one pod is running per node:

```bash
kubectl get pods -n slim -o wide
```

## Connecting Applications

Application pods connect to the standard ClusterIP service address. The `internalTrafficPolicy: Local` setting on the Service ensures the connection is always forwarded to the SLIM pod on the same node.

```python
# Python SDK — connect to the cluster-local service address
client_config = slim_bindings.new_insecure_client_config(
    "http://slim.slim.svc.cluster.local:46357"
)
conn_id = await service.connect_async(client_config)
```

No special node-targeting configuration is needed in the application.

## Host Network Mode

For use cases where pods need to communicate over the host network (e.g. when running outside of the pod network), enable `hostNetwork` and adjust the DNS policy accordingly:

```yaml
slim:
  deploymentMode: DaemonSet
  hostNetwork: true
  dnsPolicy: "ClusterFirstWithHostNet"
```

!!! warning
    Host network mode gives the SLIM pod access to the node's network interfaces. Only enable this when your workload specifically requires it.

## Rolling Updates

DaemonSet rolling updates replace pods one node at a time. `maxUnavailable: 1` (the default) ensures only one node is without a SLIM pod at any time during an update:

```yaml
slim:
  updateStrategy:
    type: RollingUpdate
    rollingUpdate:
      maxUnavailable: 1
```

!!! note
    HorizontalPodAutoscaler is not compatible with DaemonSet deployments. The replica count is always equal to the number of nodes.

## SPIRE Integration

SPIRE can be enabled for DaemonSet deployments in the same way as the standard Deployment mode. The SPIRE agent runs as a DaemonSet itself and mounts its socket into each pod.

```yaml
slim:
  deploymentMode: DaemonSet
  config:
    services:
      slim/0:
        dataplane:
          servers:
            - endpoint: "0.0.0.0:46357"
              tls:
                source:
                  type: spire
                  socket_path: unix:/tmp/spire-agent/public/api.sock

spire:
  enabled: true
```

## Next Steps

- [Kubernetes: Single Cluster](./kubernetes.md) — Standard Deployment for general-purpose single-cluster use
- [Kubernetes: Multi-Cluster](./multicluster.md) — Extend SLIM across multiple clusters
- [Authentication](../architecture/authentication.md) — SPIRE and mTLS configuration
