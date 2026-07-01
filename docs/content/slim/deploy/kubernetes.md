# Deployment: Kubernetes

This page covers deploying SLIM on a single Kubernetes cluster using the official Helm charts. Two patterns are described: a **simple** deployment for development and staging (no SPIRE, no Controller), and a **production** deployment with SPIRE-based mTLS and the Controller.

## Prerequisites

- `kubectl` configured for your cluster
- Helm 3.x
- (Production only) SPIRE or a compatible SPIFFE identity provider

## Helm Charts

SLIM publishes Helm charts to the GitHub OCI registry:

| Chart | OCI path |
|-------|----------|
| Data Plane | `oci://ghcr.io/agntcy/slim/helm/slim` |
| Controller | `oci://ghcr.io/agntcy/slim/helm/slim-control-plane` |
| SPIRE stack | `oci://ghcr.io/agntcy/slim/helm/slim-spire` |

## Simple Deployment

The simple (naive) pattern deploys a single SLIM data plane pod with no authentication and no Controller. Use this for development, staging, or when you manage routing manually via `slimctl`.

**`simple-values.yaml`:**

```yaml
slim:
  replicaCount: 1
  image:
    repository: ghcr.io/agntcy/slim
    pullPolicy: IfNotPresent
    tag: ""

  config:
    tracing:
      log_level: info
      display_thread_names: true
      display_thread_ids: true
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
  --values simple-values.yaml
```

Verify the pod is running:

```bash
kubectl get pods -n slim
```

Applications in the cluster connect to `slim.<namespace>.svc.cluster.local:46357`.

## Production Deployment

The production pattern runs 3 SLIM replicas with SPIRE mTLS and connects to the Controller for automatic route management. Kubernetes peer discovery is used so SLIM nodes automatically find each other.

### Step 1: Deploy SPIRE

```bash
helm upgrade --install slim-spire \
  oci://ghcr.io/agntcy/slim/helm/slim-spire \
  --namespace spire-system \
  --create-namespace
```

### Step 2: Deploy the Controller

**`controller-values.yaml`:**

```yaml
config:
  northbound:
    httpHost: 0.0.0.0
    httpPort: 50051
  southbound:
    httpHost: 0.0.0.0
    httpPort: 50052
  database:
    filePath: /db/controlplane.db
persistence:
  enabled: true
  size: 1Gi
spire:
  enabled: true
```

```bash
helm upgrade --install slim-control \
  oci://ghcr.io/agntcy/slim/helm/slim-control-plane \
  --namespace slim \
  --create-namespace \
  --values controller-values.yaml
```

### Step 3: Deploy SLIM Nodes

**`production-values.yaml`:**

```yaml
slim:
  replicaCount: 3

  config:
    services:
      slim/0:
        node_id: ${env:SLIM_SVC_ID}
        dataplane:
          servers:
            - endpoint: "0.0.0.0:46357"
              tls:
                insecure_skip_verify: false
                source:
                  type: spire
                  socket_path: unix:/tmp/spire-agent/public/api.sock
                  target_spiffe_id: spiffe://example.local/ns/slim/sa/slim
                ca_source:
                  type: spire
                  socket_path: unix:/tmp/spire-agent/public/api.sock
          clients: []

        controller:
          clients:
            - endpoint: "https://slim-control:50052"
              tls:
                source:
                  type: spire
                  socket_path: unix:/tmp/spire-agent/public/api.sock
                ca_source:
                  type: spire
                  socket_path: unix:/tmp/spire-agent/public/api.sock
                  trust_domains:
                    - example.local

  service:
    type: ClusterIP
    data:
      - port: 46357
        name: data-plane-0

spire:
  enabled: true
```

```bash
helm upgrade --install slim \
  oci://ghcr.io/agntcy/slim/helm/slim \
  --namespace slim \
  --values production-values.yaml
```

### Kubernetes Peer Discovery

In the production configuration, SLIM nodes discover each other automatically using Kubernetes label selectors. The Helm chart creates the necessary RBAC (`ClusterRole` + `ClusterRoleBinding`) for the SLIM service account to list and watch pods.

Set `discovery.type: kubernetes` in the SLIM config to enable this:

```yaml
services:
  slim/0:
    peers:
      discovery:
        type: kubernetes
```

### Autoscaling

The Helm chart supports Horizontal Pod Autoscaler. Enable it in your values:

```yaml
autoscaling:
  enabled: true
  minReplicas: 2
  maxReplicas: 10
  targetCPUUtilizationPercentage: 70
```

!!! note
    HPA is not available when using the DaemonSet deployment mode. See [Kubernetes: DaemonSet](./daemonset.md).

## Exposing SLIM Outside the Cluster

To allow SDK applications running outside the cluster to connect, expose the data plane service via `LoadBalancer` or `Ingress`:

```yaml
slim:
  service:
    type: LoadBalancer
    data:
      - port: 46357
        name: data-plane-0
```

For gRPC over an Ingress (nginx example):

```yaml
slim:
  ingresses:
    - enabled: true
      portName: data-plane-0
      className: "nginx"
      annotations:
        nginx.ingress.kubernetes.io/backend-protocol: "GRPC"
      hosts:
        - host: slim.example.com
          paths:
            - path: /
              pathType: Prefix
              port: "46357"
      tls:
        - secretName: slim-tls
          hosts:
            - slim.example.com
```

## Next Steps

- [Kubernetes: DaemonSet](./daemonset.md) — One SLIM pod per node for node-affine workloads
- [Kubernetes: Multi-Cluster](./multicluster.md) — Cross-cluster SLIM deployment
- [Controller Configuration Reference](../components/controller/config.md) — Full Controller config options
- [Authentication](../architecture/authentication.md) — SPIRE, JWT, and mTLS options
