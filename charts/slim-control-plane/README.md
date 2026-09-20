# slim-control-plane Helm Chart

Helm chart for the SLIM Control Plane, which manages topology and connectivity between SLIM agents.

## Installation

```bash
helm install slim-control-plane oci://ghcr.io/agntcy/helm/slim-control-plane
```

## Configuration

All parameters are set under `values.yaml`. The sections below describe each field.

---

### Basic deployment

| Parameter | Description | Default |
|-----------|-------------|---------|
| `replicaCount` | Number of pod replicas | `1` |
| `nameOverride` | Override the chart name used in resource labels | `""` |
| `fullnameOverride` | Override the full resource name | `"slim-control"` |

---

### Image

| Parameter | Description | Default |
|-----------|-------------|---------|
| `image.repository` | Container image repository | `ghcr.io/agntcy/slim/control-plane` |
| `image.tag` | Image tag. Defaults to the chart `appVersion` when empty | `""` |
| `image.pullPolicy` | Kubernetes image pull policy | `IfNotPresent` |
| `imagePullSecrets` | List of pull-secret names for private registries | `[]` |

---

### Controller configuration (`config`)

The `config` block is rendered into a `ConfigMap` and mounted as `/config.yaml` inside the container.

#### Northbound and southbound endpoints

`northbound` and `southbound` each accept a single `ServerConfig` mapping or a list of them, one entry per listener. The northbound API is the gRPC API consumed by `slimctl` and external management clients; the southbound API is used by SLIM nodes to register and receive routing updates.

Ports are templated from `service.north` / `service.south` so the addresses the control plane binds stay in step with the ones the `Service` exposes.

| Parameter | Description | Default |
|-----------|-------------|---------|
| `endpoint` | Listen address. Scheme determines transport: `ws://`/`wss://` → WebSocket, `http://`/`https://`/`unix://` or bare `host:port` → gRPC | `0.0.0.0:<port>` |
| `http2_only` | Enforce HTTP/2 only | `true` |
| `max_frame_size` | Maximum message size in MiB | `4` |
| `max_concurrent_streams` | Maximum concurrent gRPC streams per connection | `100` |
| `max_header_list_size` | Maximum header list size (bytes) | unset |
| `read_buffer_size` | Read buffer size in bytes | `1048576` |
| `write_buffer_size` | Write buffer size in bytes | `1048576` |
| `require_header_mac` | Enforce HMAC integrity validation on link negotiation headers | `true` |
| `negotiation_timeout_secs` | Timeout (seconds) for link negotiation to complete | `5` |
| `link_hmac_poll_interval_ms` | Polling interval (ms) between HMAC existence checks | `5` |

##### TLS (`tls`)

| Parameter | Description | Default |
|-----------|-------------|---------|
| `tls.insecure` | Disable TLS entirely (plaintext) | `true` in values.yaml (code default: `false`) |
| `tls.tls_version` | TLS protocol version (`tls1.2` or `tls1.3`) | `tls1.3` |
| `tls.include_system_ca_certs_pool` | Also load system root CA certificates | `true` |
| `tls.reload_interval` | Certificate reload interval (e.g. `60s`). Disabled when absent | unset |
| `tls.source.type` | Server certificate source: `file`, `pem`, or `spire` | `none` |
| `tls.source.cert` | Path to or PEM data for the server certificate (for `file`/`pem`) | — |
| `tls.source.key` | Path to or PEM data for the server private key (for `file`/`pem`) | — |
| `tls.source.socket_path` | SPIRE Workload API socket (for `spire`). Overrides `SPIFFE_ENDPOINT_SOCKET` env var | — |
| `tls.client_ca.type` | Client CA source for mTLS client certificate verification: `file`, `pem`, or `spire` | `none` |

##### Authentication (`auth`)

Auth is configured as a tagged union via `type`:

```yaml
# No authentication (default)
auth:
  type: none

# HTTP Basic authentication
auth:
  type: basic
  username: admin
  password: secret

# JWT authentication
# Server-side (verify incoming tokens): use type: decoding
auth:
  type: jwt
  claims:
    audience: ["slim"]
    issuer: "https://issuer.example.com"
    subject: "slim-client"       # optional
  duration: 3600s
  key:
    type: decoding               # encoding = sign (client), decoding = verify (server), autoresolve = auto
    algorithm: RS256
    format: pem                  # pem | jwk | jwks
    key:
      file: /path/to/public.pem  # or: data: "<inline PEM string>"

# SPIRE/SPIFFE workload identity
auth:
  type: spire
  socket_path: "unix:///run/spire/agent-sockets/api.sock"  # optional, overrides SPIFFE_ENDPOINT_SOCKET env var
  jwt_audiences: ["slim"]
  trust_domains: []
```

##### Keepalive (`keepalive`)

| Parameter | Description | Default |
|-----------|-------------|---------|
| `keepalive.max_connection_idle` | Close idle connections after this duration | `3600s` |
| `keepalive.max_connection_age` | Maximum lifetime of any connection | `7200s` |
| `keepalive.max_connection_age_grace` | Grace period after `max_connection_age` before forceful close | `300s` |
| `keepalive.time` | Interval between keepalive pings | `120s` |
| `keepalive.timeout` | Time to wait for a keepalive ping acknowledgement | `20s` |

##### Full TLS + mTLS example

```yaml
config:
  northbound:
    endpoint: "0.0.0.0:50051"
    tls:
      insecure: false
      tls_version: tls1.3
      source:
        type: file
        cert: /certs/server.crt
        key: /certs/server.key
      client_ca:
        type: file
        path: /certs/ca.crt
    auth:
      type: none
```

To use SPIRE for both TLS and auth:

```yaml
config:
  southbound:
    endpoint: "0.0.0.0:50052"
    tls:
      insecure: false
      source:
        type: spire
        socket_path: "unix:///run/spire/agent-sockets/api.sock"
    auth:
      type: spire
      socket_path: "unix:///run/spire/agent-sockets/api.sock"

spire:
  enabled: true
```

#### Database

Two backends are supported:

| Parameter | Description | Default |
|-----------|-------------|---------|
| `config.database.type` | Database backend: `sqlite` (persistent) or `inmemory` (no disk, state lost on restart) | `sqlite` |
| `config.database.path` | Path to the SQLite file inside the container (only used when `type: sqlite`) | `/db/controlplane.db` |

The `sqlite` database file lives on the PVC mounted at `/db` when `persistence.enabled: true`. When using `type: inmemory`, persistence is not needed and `persistence.enabled` can be set to `false`.

#### Tracing

| Parameter | Description | Default |
|-----------|-------------|---------|
| `config.tracing.log_level` | Log verbosity (`debug`, `info`, `warn`, `error`) | `debug` |

#### Reconciler

| Parameter | Description | Default |
|-----------|-------------|---------|
| `config.reconciler.max_requeues` | Maximum number of times a failed reconcile request is requeued | `15` |
| `config.reconciler.base_retry_delay` | Base delay for the first retry; subsequent retries use exponential backoff (base × 2^(attempt-1)) capped at 30s | `200ms` |
| `config.reconciler.reconcile_period` | How often all connected nodes are re-enqueued for a full reconciliation sweep. Set to `0s` to disable | `60s` |
| `config.reconciler.enable_orphan_detection` | When `true`, delete outgoing connections on data-plane nodes whose `link_id` is not in the control-plane DB. Useful in greenfield deployments where the CP is the sole source of truth | `false` |
| `config.reconciler.workers` | Number of concurrent worker tasks per reconciler (link and route) | `4` |

#### Topology

| Parameter | Description | Default |
|-----------|-------------|---------|
| `config.topology` | Topology definition (see modes below) | `{}` |

The control plane supports two mutually exclusive topology modes:

**API mode** (`topology: {}` or absent): topology is managed at runtime via `slimctl` or the gRPC API. Topology config (segments, links) is preserved in the DB across restarts; runtime state (nodes, routes) is rebuilt automatically when nodes reconnect.

**Config mode** (`topology` contains `links`, `segments`, or `segments-template`): the config file is the single source of truth. On every restart all state is wiped and rebuilt from the file. Topology mutation via the API is rejected.

##### Wildcard `"*"`

In both `domain` and `neighbors` fields, the value `"*"` matches all registered domains and is resolved dynamically at node registration time.

```yaml
# Full mesh — every domain connects to every other domain
config:
  topology:
    links:
      - domain: "*"
        neighbors: ["*"]

# Star — cloud connects to all, spokes only reach each other via cloud
config:
  topology:
    links:
      - domain: cloud
        neighbors: ["*"]
```

All links are **bidirectional**: declaring `cloud → edge` also creates `edge → cloud`.

##### Template variable `$domain`

`$domain` is only valid inside a `segments` definition. It causes the segment to be **instantiated once per registered domain**, with `$domain` replaced by that domain's name at registration time — it does not expand to a list of all domains simultaneously.

This is the idiomatic way to achieve per-tenant isolation where every tenant can reach a shared hub but not each other:

```yaml
# Each customer gets its own routing domain — customer-a cannot see customer-b
config:
  topology:
    segments:
      - name: segment-$domain
        links:
          - domain: cloud
            neighbors: [$domain]
```

For each domain that registers (e.g. `customer-a`, `customer-b`) the control plane generates a separate segment (`segment-customer-a`, `segment-customer-b`). Inside each segment only `cloud` ↔ that one customer are linked.

For conditional membership or grouping several matching domains into one
segment, use a MiniJinja template. `groups` contains all registered domain names
in sorted order:

```yaml
config:
  topology:
    segments-template: |
      - name: segment-customer-a
        links:
      {% for group in groups %}
      {% if group is startingwith("customer-a-") %}
          - domain: {{ "{{ group }}" }}
            neighbors: [customer-a]
      {% endif %}
      {% endfor %}
```

Use `tojson` for interpolated names so they remain safely quoted YAML scalars.
The extra outer `{{ "..." }}` in Helm values preserves the inner MiniJinja
expression when the chart evaluates `config` with Helm's `tpl` function.
The `links`, `segments`, and `segments-template` fields are mutually exclusive.

Named segments (explicit multi-tenant isolation without the template):

```yaml
config:
  topology:
    segments:
      - name: tenant-1
        links:
          - domain: cloud
            neighbors: [cluster-a]
      - name: tenant-2
        links:
          - domain: cloud
            neighbors: [cluster-b, cluster-c]
```

When `segments` is present, any top-level `topology.links` is ignored — segments fully control both link creation and route expansion.

---

### SPIRE

SPIRE provides workload identity (SVID certificates) for mTLS between SLIM components.

| Parameter | Description | Default |
|-----------|-------------|---------|
| `spire.enabled` | Deploy a `ClusterSPIFFEID` resource and mount the SPIRE agent socket | `false` |
| `spire.trustedDomains` | List of external SPIFFE trust domains to federate with | `[]` |

When `spire.enabled: true` the chart:
- Mounts the SPIRE agent socket from the host at `/run/spire/agent-sockets`
- Creates a `ClusterSPIFFEID` that assigns an SVID to the control-plane pod

To fully enable mTLS you also need to update `config.southbound` (and/or
`config.northbound`):

```yaml
config:
  southbound:
    - endpoint: "0.0.0.0:{{ (index .Values.service.south 0).port }}"
      tls:
        insecure: false
        source:
          type: spire
          socket_path: "unix:///run/spire/agent-sockets/api.sock"
      auth:
        type: spire
        socket_path: "unix:///run/spire/agent-sockets/api.sock"

spire:
  enabled: true
  trustedDomains:
    - cluster-a.example.org
```

---

### Environment variables

| Parameter | Description | Default |
|-----------|-------------|---------|
| `env` | List of extra environment variables injected into the container | HTTP host bindings for NB/SB APIs |

Default env vars bind the HTTP management endpoints to all interfaces:

```yaml
env:
  - name: NB_API_HTTP_HOST
    value: "0.0.0.0"
  - name: SB_API_HTTP_HOST
    value: "0.0.0.0"
```

---

### Service account

| Parameter | Description | Default |
|-----------|-------------|---------|
| `serviceAccount.create` | Create a dedicated `ServiceAccount` | `true` |
| `serviceAccount.automount` | Automount the service account token | `true` |
| `serviceAccount.annotations` | Annotations to add to the service account | `{}` |
| `serviceAccount.name` | Name of the service account. Auto-generated when empty and `create: true` | `""` |

---

### Pod configuration

| Parameter | Description | Default |
|-----------|-------------|---------|
| `podAnnotations` | Extra annotations added to the pod | `{}` |
| `podLabels` | Extra labels added to the pod | `{}` |
| `podSecurityContext` | Pod-level security context (`fsGroup`, etc.) | `{}` |
| `securityContext` | Container-level security context (`runAsUser`, `readOnlyRootFilesystem`, etc.) | `{}` |

---

### Service

The chart exposes one named port per control-plane listener on a single
Kubernetes `Service`.

| Parameter | Description | Default |
|-----------|-------------|---------|
| `service.type` | Kubernetes service type | `ClusterIP` |
| `service.north` | List of northbound ports to expose, one entry per northbound listener. Each entry takes `port` and an optional `name` (max 15 chars); the name defaults to `north-{index}` | `[{port: 50051, name: north}]` |
| `service.south` | List of southbound ports to expose, one entry per southbound listener. Name defaults to `south-{index}` | `[{port: 50052, name: south}]` |

To serve an API on several addresses, add an entry here and a matching one under
`config.northbound` / `config.southbound`, referencing the port by index the same
way the default values do:

```yaml
service:
  north:
    - port: 50051
      name: north
    - port: 50451
      name: north-tls

config:
  northbound:
    - endpoint: "0.0.0.0:{{ (index .Values.service.north 0).port }}"
      tls:
        insecure: true
    - endpoint: "0.0.0.0:{{ (index .Values.service.north 1).port }}"
      tls:
        insecure: false
        source:
          type: spire
          socket_path: "unix:///run/spire/agent-sockets/api.sock"
```

An Ingress targets the bound's first port.

---

### Persistence

The SQLite database is stored on a `PersistentVolumeClaim` mounted at `/db`.

| Parameter | Description | Default |
|-----------|-------------|---------|
| `persistence.enabled` | Create and mount a PVC for the database | `true` |
| `persistence.existingClaim` | Use an existing PVC instead of creating a new one | `""` |
| `persistence.accessModes` | PVC access modes | `[ReadWriteOnce]` |
| `persistence.size` | Storage size requested by the PVC | `1Gi` |
| `persistence.storageClass` | StorageClass name. Uses the cluster default when empty | `""` |

---

### Ingress

Two independent `Ingress` objects can be created — one per port. Both share the same structure.

#### Northbound ingress (`ingressNorth`)

| Parameter | Description | Default |
|-----------|-------------|---------|
| `ingressNorth.enabled` | Create an Ingress for the northbound API | `false` |
| `ingressNorth.servicePort` | Which listener to expose, by port name or number from `service.north`. Unset selects the first listener | `""` |
| `ingressNorth.className` | `ingressClassName` value | `""` |
| `ingressNorth.annotations` | Ingress annotations (e.g. `nginx.ingress.kubernetes.io/backend-protocol: GRPC`) | see values |
| `ingressNorth.hosts` | List of host rules (`host`, `paths[].path`, `paths[].pathType`) | example host |
| `ingressNorth.tls` | TLS configuration (`secretName`, `hosts`) | `[]` |
| `ingressNorth.basicAuth.enabled` | Enable nginx basic-auth via annotation | `false` |
| `ingressNorth.basicAuth.existingBasicAuthSecret` | Name of the Secret containing the htpasswd file | `""` |
| `ingressNorth.basicAuth.realm` | Basic-auth realm string | `""` |

#### Southbound ingress (`ingressSouth`)

Same fields as `ingressNorth` (including `servicePort`), applies to the southbound API.

#### Exposing only some listeners

An Ingress publishes exactly one listener — the one `servicePort` names, or the
first by default. With `service.type: ClusterIP` the remaining listeners are
reachable only inside the cluster, which is how you keep one address public and
another internal:

```yaml
service:
  type: ClusterIP
  north:
    - port: 50051          # in-cluster only
      name: north
    - port: 50451          # published below
      name: north-tls

ingressNorth:
  enabled: true
  servicePort: north-tls
```

A `servicePort` that matches no listener fails rendering with the available names
listed, rather than producing an Ingress that 503s.

Note that `service.type: LoadBalancer` or `NodePort` exposes **every** port on the
Service regardless of the Ingress, so the in-cluster-only property depends on
`ClusterIP`. Splitting listeners across trust boundaries with a LoadBalancer needs
a second Service selecting the same pod, which this chart does not create.

---

### Resources

| Parameter | Description | Default |
|-----------|-------------|---------|
| `resources` | CPU/memory requests and limits for the container | `{}` (no limits) |

Example:

```yaml
resources:
  limits:
    cpu: 500m
    memory: 256Mi
  requests:
    cpu: 100m
    memory: 128Mi
```

---

### Probes

Liveness and readiness probes are not configured by default. Add them under:

```yaml
livenessProbe:
  grpc:
    port: 50051
readinessProbe:
  grpc:
    port: 50051
```

---

### Autoscaling

| Parameter | Description | Default |
|-----------|-------------|---------|
| `autoscaling.enabled` | Enable a `HorizontalPodAutoscaler` | `false` |
| `autoscaling.minReplicas` | Minimum replica count | `1` |
| `autoscaling.maxReplicas` | Maximum replica count | `100` |
| `autoscaling.targetCPUUtilizationPercentage` | Target CPU utilization (%) for scaling | `80` |
| `autoscaling.targetMemoryUtilizationPercentage` | Target memory utilization (%) for scaling | not set |

When `autoscaling.enabled: true` the top-level `replicaCount` is ignored.

---

### Scheduling

| Parameter | Description | Default |
|-----------|-------------|---------|
| `nodeSelector` | Node labels to constrain pod placement | `{}` |
| `tolerations` | Tolerations for tainted nodes | `[]` |
| `affinity` | Affinity and anti-affinity rules | `{}` |

---

### Extra volumes

| Parameter | Description | Default |
|-----------|-------------|---------|
| `volumes` | Additional volumes to add to the pod spec | `[]` |
| `volumeMounts` | Additional volume mounts to add to the container | `[]` |
