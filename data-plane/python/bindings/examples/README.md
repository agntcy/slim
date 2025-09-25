# SLIM Python Binding Examples

This directory contains runnable example programs that demonstrate how to use the SLIM Python bindings for different communication patterns:

- Anycast: Send to one of many eligible receivers (chosen pseudo-randomly / load-balanced)
- Unicast: Send to a specific peer
- Multicast: Send to a group (moderator-driven membership)
- With or without MLS (Messaging Layer Security) for end-to-end encryption

You can run these examples:
1. Locally (standalone) using a shared secret for establishing trust
2. In a Kubernetes cluster using static JWTs issued by SPIRE

---

## Contents

- [Prerequisites](#prerequisites)
- [Concepts at a Glance](#concepts-at-a-glance)
- [Quick Start (Standalone)](#quick-start-standalone)
  - [Step 1: Start the SLIM node](#step-1-start-the-slim-node)
  - [Step 2: Anycast Example](#step-2-anycast-example)
  - [Step 3: Unicast Example (MLS and non-MLS)](#step-3-unicast-example-mls-and-non-mls)
  - [Step 4: Multicast Example](#step-4-multicast-example)
- [Interpreting the Output](#interpreting-the-output)
- [Modifying the Examples](#modifying-the-examples)
- [Running in Kubernetes (SPIRE / JWT)](#running-in-kubernetes-spire--jwt)
- [Troubleshooting](#troubleshooting)
- [Next Steps](#next-steps)

---

## Prerequisites

- Python environment compatible with the SLIM bindings (see project-level docs for version specifics)
- Installed Task runner: https://taskfile.dev/docs/installation
- Multiple terminals (or a terminal multiplexer) to run concurrent peers
- The Rust toolchain (for building the bindings, if not already done)
- Helm and kubectl (for the Kubernetes section)

To view the underlying command a task executes, add `-v`:
```bash
task -v python:example:server
```

---

## Concepts at a Glance

| Pattern   | Delivery Semantics                                   | Typical Use Case                       | MLS Support |
|-----------|-------------------------------------------------------|----------------------------------------|-------------|
| Anycast   | One of N receivers (probabilistic / load-sharing)     | Stateless workers, load balancing      | Absent    |
| Unicast   | Exactly one intended peer                            | Direct messaging / RPC-like flows      | Present    |
| Multicast | All members of a moderator-defined group             | Group coordination / pub-sub-like      | Present |

MLS (Messaging Layer Security) provides end-to-end encryption and group state management. Non-MLS modes may still use channel protection in the links between SLIM nodes, but are not full E2E group cryptographic sessions.

---

## Quick Start (Standalone)

### Step 1: Start the SLIM node

This launches the central coordination / rendezvous node used by the examples:

```bash
task python:example:server
```

Leave this running.

### Step 2: Anycast Example

Open two terminals and start two `alice` instances:

```bash
task python:example:p2p:alice
Agntcy/ns/alice/9429169046562807017          Created app
Agntcy/ns/alice/9429169046562807017          Connected to http://localhost:46357
Agntcy/ns/alice/9429169046562807017          waiting for new session to be established
```

Run the same command again in a second terminal (the numeric ID will differ).

Then start `bob` (the sender) in a third terminal:

```bash
task python:example:p2p:anycast:bob
Agntcy/ns/bob/14478648478491643199           Created app
Agntcy/ns/bob/14478648478491643199           Connected to http://localhost:46357
Agntcy/ns/bob/14478648478491643199           Sent message hey there - 1/10:
Agntcy/ns/bob/14478648478491643199           received (from session 2178188337): hey there from agntcy/ns/alice/9429169046562807017
Agntcy/ns/bob/14478648478491643199           Sent message hey there - 2/10:
Agntcy/ns/bob/14478648478491643199           received (from session 2178188337): hey there from agntcy/ns/alice/7694050331430527832
...
```

You will see successive messages being received by both `alice` instances — distribution is effectively anycast.

### Step 3: Unicast Example (MLS and non-MLS)

In a new terminal, experiment with the unicast variants. Only one chosen `alice` receives each message.

```bash
# With MLS enabled
task python:example:p2p:unicast:mls:bob
Agntcy/ns/bob/1309096762860029159            Created app
Agntcy/ns/bob/1309096762860029159            Connected to http://localhost:46357
Agntcy/ns/bob/1309096762860029159            Sent message hey there - 1/10:
Agntcy/ns/bob/1309096762860029159            received (from session 2689354079): hey there from agntcy/ns/alice/9429169046562807017
...
```

```bash
# Without MLS
task python:example:p2p:unicast:no-mls:bob
Agntcy/ns/bob/1309096762860029159            Created app
Agntcy/ns/bob/1309096762860029159            Connected to http://localhost:46357
Agntcy/ns/bob/1309096762860029159            Sent message hey there - 1/10:
Agntcy/ns/bob/1309096762860029159            received (from session 2689354079): hey there from agntcy/ns/alice/9429169046562807017
...
```

Result: only one `alice` instance receives each message — this is deterministic selection vs anycast's pseudo-random distribution.

### Step 4: Multicast Example

Stop the prior `alice` processes (to reduce noise). Start two multicast clients:

```bash
# Client 1
task python:example:multicast:client-1
Agntcy/ns/client-1/7183465155134805761       Created app
Agntcy/ns/client-1/7183465155134805761       Connected to http://localhost:46357
Agntcy/ns/client-1                           -> Waiting for session...
```

```bash
# Client 2
task python:example:multicast:client-2
Agntcy/ns/client-2/597424660555802635        Created app
Agntcy/ns/client-2/597424660555802635        Connected to http://localhost:46357
Agntcy/ns/client-2                           -> Waiting for session...
```

In a third terminal, run the moderator (it creates the group and invites members):

```bash
task python:example:multicast:moderator
Agntcy/ns/moderator/1639897447396238611      Created app
Agntcy/ns/moderator/1639897447396238611      Connected to http://localhost:46357
Creating new multicast session (moderator)... 169ca82eb17d6bc2/eef9769a4c6990d1/fc9bbc406957794b/ffffffffffffffff (agntcy/ns/moderator/ffffffffffffffff)
agntcy/ns/moderator -> add 169ca82eb17d6bc2/eef9769a4c6990d1/58ec40d7c837e0b9/ffffffffffffffff (agntcy/ns/client-1/ffffffffffffffff) to the group
agntcy/ns/moderator -> add 169ca82eb17d6bc2/eef9769a4c6990d1/b521a3788f1267a8/ffffffffffffffff (agntcy/ns/client-2/ffffffffffffffff) to the group
message> hey guys
```

Typical client output (example):

```bash
Agntcy/ns/client-2                           -> Received message from 169ca82eb17d6bc2/eef9769a4c6990d1/fc9bbc406957794b/16c2160a341f6513 (agntcy/ns/moderator/16c2160a341f6513): hey guys
```

In the baseline example only the moderator sends; you can enable bidirectional publishing (see [Modifying the Examples](#modifying-the-examples)).

---

## Interpreting the Output

- `Agntcy/ns/<name>/<numeric_id>`: Identifies the logical application instance.
- `Created app / Connected`: Lifecycle events for the client bootstrap.
- `Waiting for session...`: Peer is idle until a session (unicast/multicast) is formed.
- Session or group IDs (hex fragments) represent cryptographic / routing context.
- MLS-enabled logs may include group or epoch transitions (depending on verbosity).

---

## Modifying the Examples

Ideas:
1. Allow multicast clients to send messages:
   - Locate the event loop reading stdin in the moderator example and replicate a similar loop in the client implementation.
2. Add structured logging (JSON) for easier parsing.
3. Inject artificial delays to study ordering and fairness.
4. Add metrics counters (messages sent / received per peer).
5. Switch between MLS and non-MLS to profile overhead.

---

## Running in Kubernetes (SPIRE / JWT)

This section shows how to run the examples inside a Kubernetes cluster where workload identity is provided by SPIRE. You will:
1. Create a local KIND cluster (with an in-cluster image registry).
2. Install SPIRE (server + agents).
3. Build and push SLIM images to the local registry.
4. Deploy the SLIM node (control / rendezvous component).
5. Deploy two distinct SLIM client workloads, each with its own ServiceAccount (and thus its own SPIFFE ID).
6. Run the unicast example using JWT-based authentication derived from SPIRE.

If you already have a Kubernetes cluster or an existing SPIRE deployment, you can adapt only the relevant subsections.

### Create a KIND cluster with a local image registry

The helper script below provisions a KIND cluster and configures a local registry (localhost:5001) that the cluster’s container runtime can pull from:

```bash
curl -L https://kind.sigs.k8s.io/examples/kind-with-registry.sh | sh
```

### Install SPIRE (server + CRDs + agents)

```bash
helm upgrade --install -n spire-server spire-crds spire-crds --repo https://spiffe.github.io/helm-charts-hardened/ --create-namespace
helm upgrade --install -n spire-server spire spire --repo https://spiffe.github.io/helm-charts-hardened/
```

Wait for the SPIRE components to become ready:

```bash
kubectl get pods -n spire-server
```

All pods should reach Running/READY status before proceeding.

### SPIFFE ID strategy

The default SPIRE server Helm chart installs a Cluster SPIFFE ID controller object (`spire-server-spire-default`) that issues workload identities following the pattern:

```
spiffe://domain.test/ns/<namespace>/sa/<service-account>
```

We will rely on that default. If you need more granular issuance (specific label selectors, different trust domain, etc.), consult the
[ClusterSPIFFEID documentation](https://github.com/spiffe/spire-controller-manager/blob/main/docs/clusterspiffeid-crd.md).

### Build SLIM images (node + examples)

You can use pre-built images if available; here we build and push fresh ones to the local registry:

```bash
REPO_ROOT=$(git rev-parse --show-toplevel)
pushd "${REPO_ROOT}"
IMAGE_REPO=localhost:5001 docker bake slim && docker push localhost:5001/slim:latest
IMAGE_REPO=localhost:5001 docker bake bindings-examples && docker push localhost:5001/bindings-examples:latest
popd
```

Verify they are present (optional):
```bash
crane ls localhost:5001 | grep slim
```

### Deploy the SLIM node

```bash
REPO_ROOT=$(git rev-parse --show-toplevel)
pushd "${REPO_ROOT}/charts"
helm install \
  --create-namespace \
  -n slim \
  slim ./slim \
  --set slim.image.repository=localhost:5001/slim \
  --set slim.image.tag=latest
```

Confirm the pod is running:
```bash
kubectl get pods -n slim
```

### Deploy client configuration (ConfigMap)

We first provide a config for `spiffe-helper`, which retrieves SVIDs/JWTs from the SPIRE agent and writes them to disk. Key fields:
- `agent_address`: Path to the SPIRE agent API socket.
- `cert_dir`: Where artifacts (cert/key/bundles/JWTs) are written.
- `jwt_svids`: Audience + output filename for requested JWT SVIDs.
- `daemon_mode = true`: Run continuously to renew material.

```bash
kubectl apply -f - <<EOF
apiVersion: v1
kind: ConfigMap
metadata:
  name: spire-helper-slim-client
  labels:
    app.kubernetes.io/name: slim-client
data:
  helper.conf: |
    agent_address =  "/run/spire/agent-sockets/api.sock"
    cmd = ""
    cmd_args = ""
    cert_dir = "/svids"
    renew_signal = ""
    svid_file_name = "tls.crt"
    svid_key_file_name = "tls.key"
    svid_bundle_file_name = "svid_bundle.pem"
    jwt_bundle_file_name = "key.jwt"
    cert_file_mode = 0600
    key_file_mode = 0600
    jwt_svid_file_mode = 0600
    jwt_bundle_file_mode = 0600
    jwt_svids = [{jwt_audience="slim-demo", jwt_svid_file_name="jwt_svid.token"}]
    daemon_mode = true
EOF
```

### Deploy two distinct clients (separate ServiceAccounts = separate SPIFFE IDs)

Each Deployment:
- Has its own ServiceAccount (`slim-client-a`, `slim-client-b`).
- Mounts the SPIRE agent socket from the host (in KIND, agent runs as a DaemonSet).
- Runs `spiffe-helper` sidecar to continuously refresh identities.
- Runs a placeholder `slim-client` container (sleep) you can exec into.

```bash
kubectl apply -f - <<EOF
apiVersion: v1
kind: ServiceAccount
metadata:
  name: slim-client-a
  labels:
    app.kubernetes.io/name: slim-client
    app.kubernetes.io/component: client-a
---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: slim-client-b
  labels:
    app.kubernetes.io/name: slim-client
    app.kubernetes.io/component: client-b
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: slim-client-a
  labels:
    app.kubernetes.io/name: slim-client
    app.kubernetes.io/component: client-a
spec:
  replicas: 1
  selector:
    matchLabels:
      app.kubernetes.io/name: slim-client
      app.kubernetes.io/component: client-a
  template:
    metadata:
      labels:
        app.kubernetes.io/name: slim-client
        app.kubernetes.io/component: client-a
    spec:
      serviceAccountName: slim-client-a
      securityContext: {}
      containers:
        - name: spiffe-helper
          image: ghcr.io/spiffe/spiffe-helper:0.10.0
          imagePullPolicy: IfNotPresent
          args: [ "-config", "config/helper.conf" ]
          volumeMounts:
            - name: config-volume
              mountPath: /config/helper.conf
              subPath: helper.conf
            - name: spire-agent-socket
              mountPath: /run/spire/agent-sockets
              readOnly: false
            - name: svids-volume
              mountPath: /svids
              readOnly: false
        - name: slim-client
          securityContext: {}
          image: "localhost:5001/bindings-examples:latest"
          imagePullPolicy: Always
          command: ["sleep"]
          args: ["infinity"]
          resources: {}
          volumeMounts:
            - name: svids-volume
              mountPath: /svids
              readOnly: false
            - name: config-volume
              mountPath: /config/helper.conf
              subPath: helper.conf
      volumes:
        - name: spire-agent-socket
          hostPath:
            path: /run/spire/agent-sockets
            type: Directory
        - name: config-volume
          configMap:
            name: spire-helper-slim-client
        - name: svids-volume
          emptyDir: {}
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: slim-client-b
  labels:
    app.kubernetes.io/name: slim-client
    app.kubernetes.io/component: client-b
spec:
  replicas: 1
  selector:
    matchLabels:
      app.kubernetes.io/name: slim-client
      app.kubernetes.io/component: client-b
  template:
    metadata:
      labels:
        app.kubernetes.io/name: slim-client
        app.kubernetes.io/component: client-b
    spec:
      serviceAccountName: slim-client-b
      securityContext: {}
      containers:
        - name: spiffe-helper
          image: ghcr.io/spiffe/spiffe-helper:0.10.0
          imagePullPolicy: IfNotPresent
          args: [ "-config", "config/helper.conf" ]
          volumeMounts:
            - name: config-volume
              mountPath: /config/helper.conf
              subPath: helper.conf
            - name: spire-agent-socket
              mountPath: /run/spire/agent-sockets
              readOnly: false
            - name: svids-volume
              mountPath: /svids
              readOnly: false
        - name: slim-client
          securityContext: {}
          image: "localhost:5001/bindings-examples:latest"
          imagePullPolicy: Always
          command: ["sleep"]
          args: ["infinity"]
          resources: {}
          volumeMounts:
            - name: svids-volume
              mountPath: /svids
              readOnly: false
            - name: config-volume
              mountPath: /config/helper.conf
              subPath: helper.conf
      volumes:
        - name: spire-agent-socket
          hostPath:
            path: /run/spire/agent-sockets
            type: Directory
        - name: config-volume
          configMap:
            name: spire-helper-slim-client
        - name: svids-volume
          emptyDir: {}
EOF
```

Check that both pods are running:
```bash
kubectl get pods -l app.kubernetes.io/name=slim-client -o wide
```

You can inspect each pod’s SPIFFE ID with:
```bash
kubectl exec -it <pod> -c spiffe-helper -- cat /svids/jwt_svid.token | head -1
```

### Run the unicast example (inside the cluster)

Enter the first client pod (receiver):

```bash
kubectl exec -c slim-client -it $(kubectl get pods -l app.kubernetes.io/component=client-a -o jsonpath="{.items[0].metadata.name}") -- /bin/bash
```

Verify the identity artifacts:

```bash
ls -l /svids
```

Run the receiver:

```bash
/app/bin/unicast --slim '{"endpoint": "http://slim.slim:46357", "tls": {"insecure": true}}' \
  --jwt /svids/jwt_svid.token \
  --spire-trust-bundle /svids/key.jwt \
  --local agntcy/example/receiver \
  --audience slim-demo
```

Open a second shell for the sender:

```bash
kubectl exec -c slim-client -it $(kubectl get pods -l app.kubernetes.io/component=client-b -o jsonpath="{.items[0].metadata.name}") -- /bin/bash
```

Run the sender:

```bash
/app/bin/unicast --slim '{"endpoint": "http://slim.slim:46357", "tls": {"insecure": true}}' \
  --jwt /svids/jwt_svid.token \
  --spire-trust-bundle /svids/key.jwt \
  --audience slim-demo \
  --local agntcy/example/sender \
  --remote agntcy/example/receiver \
  --mls-enabled \
  --message "hey there"
```

Sample output (abridged):

```
Agntcy/example/sender/...  Created app
Agntcy/example/sender/...  Connected to http://slim.slim:46357
Agntcy/example/sender/...  Sent message hey there - 1/10:
Agntcy/example/sender/...  received (from session ...): hey there from agntcy/example/receiver/...
```

At this point the two workloads are securely exchanging messages authenticated by SPIRE-issued identities and authorized via JWT claims (audience + expiration). The MLS flag demonstrates establishing an end-to-end encrypted channel.

---


## Summary

- Anycast: Load-balanced delivery across identical service instances.
- Unicast: Direct, deterministic peer-to-peer communication.
- Multicast: Group-oriented, moderator-managed distribution.
- MLS (where enabled): Provides end-to-end encrypted group state and forward secrecy benefits.

Experiment, extend, and adapt—these examples are a foundation for building secure, flexible distributed applications with SLIM.

Happy hacking!
