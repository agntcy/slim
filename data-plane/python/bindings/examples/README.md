# SLIM Python Binding Examples

This directory contains runnable example programs that demonstrate how to use the SLIM Python bindings for different communication patterns:

- Point2Point: Selects one of many eligible receivers when the session is created,
  and sends all the messages to the same specific peer
- Group: Send to a group (moderator-driven membership)
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
    - [Step 2: Point2Point Example (MLS and non-MLS)](#step-2-p2p-example-mls-and-non-mls)
    - [Step 3: Group Example](#step-3-group-example)
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

| Pattern     | Delivery Semantics                                                                                                       | Typical Use Case                  | MLS Support |
| ----------- | ------------------------------------------------------------------------------------------------------------------------ | --------------------------------- | ----------- |
| Point2Point | Selects one of many eligible receivers when the session is created, and sends all the messages to the same specific peer | Direct messaging / RPC-like flows | Present     |
| Group       | All members of a moderator-defined group                                                                                 | Group coordination / pub-sub-like | Present     |

MLS (Messaging Layer Security) provides end-to-end encryption and group state management. Non-MLS modes may still use channel protection in the links between SLIM nodes, but are not full E2E group cryptographic sessions.

---

## Quick Start (Standalone)

### Step 1: Start the SLIM node

This launches the central coordination / rendezvous node used by the examples:

```bash
task python:example:server
```

Leave this running.

### Step 2: PointToPoint Example (MLS and non-MLS)

Open two terminals and start two `alice` instances:

```bash
task python:example:p2p:alice
```

The output will look like:

```bash
Agntcy/ns/alice/9429169046562807017          Created app
Agntcy/ns/alice/9429169046562807017          Connected to http://localhost:46357
Agntcy/ns/alice/9429169046562807017          waiting for new session to be established
```

Run the same command again in a second terminal (the numeric ID will differ).

In a new terminal, experiment with the PointToPoint variants. Only one chosen `alice` receives each message.

```bash
# With MLS enabled
task python:example:p2p:mls:bob
Agntcy/ns/bob/1309096762860029159            Created app
Agntcy/ns/bob/1309096762860029159            Connected to http://localhost:46357
Agntcy/ns/bob/1309096762860029159            Sent message hey there - 1/10:
Agntcy/ns/bob/1309096762860029159            received (from session 2689354079): hey there from agntcy/ns/alice/9429169046562807017
...
```

```bash
# Without MLS
task python:example:p2p:no-mls:bob
Agntcy/ns/bob/1309096762860029159            Created app
Agntcy/ns/bob/1309096762860029159            Connected to http://localhost:46357
Agntcy/ns/bob/1309096762860029159            Sent message hey there - 1/10:
Agntcy/ns/bob/1309096762860029159            received (from session 2689354079): hey there from agntcy/ns/alice/9429169046562807017
...
```

Result: only one `alice` instance receives each message.
For more info details the session protocols, see https://github.com/agntcy/slim/blob/main/data-plane/python/bindings/SESSION.md#slim-sessions

### Step 3: Group Example

Stop the prior `alice` and `bob` processes (to reduce noise). Start two group clients:

```bash
# Client 1
task python:example:group:client-1
Agntcy/ns/client-1/7183465155134805761       Created app
Agntcy/ns/client-1/7183465155134805761       Connected to http://localhost:46357
Agntcy/ns/client-1                           -> Waiting for session...
```

```bash
# Client 2
task python:example:group:client-2
Agntcy/ns/client-2/597424660555802635        Created app
Agntcy/ns/client-2/597424660555802635        Connected to http://localhost:46357
Agntcy/ns/client-2                           -> Waiting for session...
```

In a third terminal, run the moderator (it creates the group and invites members):

```bash
task python:example:group:moderator
Agntcy/ns/moderator/1639897447396238611      Created app
Agntcy/ns/moderator/1639897447396238611      Connected to http://localhost:46357
Creating new group session (moderator)... 169ca82eb17d6bc2/eef9769a4c6990d1/fc9bbc406957794b/ffffffffffffffff (agntcy/ns/moderator/ffffffffffffffff)
agntcy/ns/moderator -> add 169ca82eb17d6bc2/eef9769a4c6990d1/58ec40d7c837e0b9/ffffffffffffffff (agntcy/ns/client-1/ffffffffffffffff) to the group
agntcy/ns/moderator -> add 169ca82eb17d6bc2/eef9769a4c6990d1/b521a3788f1267a8/ffffffffffffffff (agntcy/ns/client-2/ffffffffffffffff) to the group
message> hey guys
```

Typical client output (example):

```bash
Agntcy/ns/client-2                           -> Received message from 169ca82eb17d6bc2/eef9769a4c6990d1/fc9bbc406957794b/16c2160a341f6513 (agntcy/ns/moderator/16c2160a341f6513): hey guys
```

In the baseline example only the moderator sends, but the session is bidirectional
and all participants can send messages on the shared channel (see [Modifying the Examples](#modifying-the-examples)).

---

## Running in Kubernetes (SPIRE / JWT)

This section shows how to run the examples inside a Kubernetes cluster where workload identity is provided by SPIRE. You will:

1. Create a local KIND cluster (with an in-cluster image registry).
2. Install SPIRE (server + agents).
3. Build and push SLIM images to the local registry.
4. Deploy the SLIM node (control / rendezvous component).
5. Deploy two distinct SLIM client workloads, each with its own ServiceAccount (and thus its own SPIFFE ID).
6. Run the PointToPoint example using JWT-based authentication derived from SPIRE.

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
IMAGE_REPO=localhost:5001 docker buildx bake slim --load && docker push localhost:5001/slim:latest
IMAGE_REPO=localhost:5001 docker buildx bake bindings-examples --load && docker push localhost:5001/bindings-examples:latest
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
        - name: slim-client
          securityContext: {}
          image: "localhost:5001/bindings-examples:latest"
          imagePullPolicy: Always
          command: ["sleep"]
          args: ["infinity"]
          resources: {}
          volumeMounts:
            - name: spire-agent-socket
              mountPath: /run/spire/agent-sockets
              readOnly: false
      volumes:
        - name: spire-agent-socket
          hostPath:
            path: /run/spire/agent-sockets
            type: Directory
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
        - name: slim-client
          securityContext: {}
          image: "localhost:5001/bindings-examples:latest"
          imagePullPolicy: Always
          command: ["sleep"]
          args: ["infinity"]
          resources: {}
          volumeMounts:
            - name: spire-agent-socket
              mountPath: /run/spire/agent-sockets
              readOnly: false
      volumes:
        - name: spire-agent-socket
          hostPath:
            path: /run/spire/agent-sockets
            type: Directory
EOF
```

Check that both pods are running:

```bash
kubectl get pods -l app.kubernetes.io/name=slim-client -o wide
```

You can inspect each pod’s SPIFFE ID with:

```bash
POD_NAME=$(kubectl get pods -l app.kubernetes.io/component=client-a -o jsonpath="{.items[0].metadata.name}")
kubectl exec -c slim-client -it ${POD_NAME} -- ls -l /svids
```

### Run the PointToPoint example (inside the cluster)

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
/app/bin/p2p --slim '{"endpoint": "http://slim.slim:46357", "tls": {"insecure": true}}' \
  --spire-socket-path /run/spire/agent-sockets/api.sock \
  --spire-jwt-audience slim-demo \
  --local agntcy/example/receiver \
```

Open a second shell for the sender:

```bash
kubectl exec -c slim-client -it $(kubectl get pods -l app.kubernetes.io/component=client-b -o jsonpath="{.items[0].metadata.name}") -- /bin/bash
```

Run the sender:

```bash
/app/bin/p2p --slim '{"endpoint": "http://slim.slim:46357", "tls": {"insecure": true}}' \
  --spire-socket-path /run/spire/agent-sockets/api.sock \
  --spire-jwt-audience slim-demo \
  --local agntcy/example/sender \
  --remote agntcy/example/receiver \
  --enable-mls \
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
