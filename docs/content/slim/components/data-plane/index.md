# SLIM Data Plane

The SLIM Data Plane is the core routing engine of a SLIM deployment. It is a high-performance Rust process that accepts connections from applications and other SLIM nodes, routes messages by hierarchical name, and enforces authentication at every connection.

## Responsibilities

- **Message routing** — delivers messages to named endpoints using anycast (any instance of a service) or unicast (a specific instance) addressing
- **Connection management** — accepts inbound connections from applications over gRPC or WebSocket; maintains outbound connections to peer SLIM nodes to form a mesh
- **Transport security** — secures connections with TLS or mTLS between applications and nodes, and between peer nodes
- **Controller integration** — registers with the SLIM Controller and receives route and connection configuration updates via the Controller's southbound API
- **Observability** — exports traces and metrics via OpenTelemetry

## Architecture

Internally the Data Plane is composed of two main layers:

- **Service layer** — bootstraps the process and exposes the public API for creating connections and servers. It owns a `MessageProcessor`, through which all transport and routing operations flow.
- **Datapath layer** — the `MessageProcessor` runs the gRPC and WebSocket servers and owns a `Forwarder`. The `Forwarder` in turn owns the `ConnectionTable` (all active connections) and the `SubscriptionTable` (name-to-connection mappings that drive routing).

### Message Flow

1. A connection task receives a message and performs basic validation: HMAC integrity check and TTL decrement. Messages that fail validation or whose TTL reaches zero are dropped.
2. The message type determines dispatch: **subscribe** updates the subscription table, **publish** triggers a routing lookup, **link** (peer control) updates the connection table.
3. For publish messages the `Forwarder` looks up subscriptions matching the destination name. If the message's fanout field is `1` it picks a single matching connection (anycast); otherwise it delivers to all matching connections (multicast).

### Connection Types

The Data Plane distinguishes four connection categories:

| Type | Description |
|------|-------------|
| **Local** | In-process applications connected via the SDK |
| **Edge** | First-hop connection from an application to the SLIM network (gRPC or WebSocket) |
| **Peer** | Other Data Plane nodes in the same deployment (replica peers) |
| **Remote** | Data Plane nodes in remote deployments (inter-cluster links) |

Peer connections enforce a one-hop rule: messages received from a peer are never forwarded to another peer, preventing routing loops.

### Link Negotiation

Before a new peer connection is inserted into the routing table, both sides perform an X25519 ECDH key exchange to derive a shared HMAC session key. This key is used to sign and verify the integrity of control messages on that link.

### Routing Table Design

Both the connection table and subscription table use copy-on-write data structures (ArcSwap) so that the read path — which runs on every message — is lock-free. Writes (subscription updates, connection changes) replace the table atomically without blocking in-flight reads.

### Peer Discovery and Sync

When a Data Plane node connects to a peer, it performs a full routing table exchange so both sides converge on the same view of the network. Peer addresses are discovered either from static configuration or from Kubernetes EndpointSlices, enabling automatic peer formation when running as a Deployment or DaemonSet.

## In This Section

- [Installation Guide](./install.md) — Build or download the SLIM Data Plane binary and run it
- [Configuration Reference](./config.md) — Full YAML configuration reference

## Related

- [Architecture](../../architecture/index.md) — How the Data Plane fits into the overall SLIM architecture
- [Naming](../../architecture/naming.md) — The hierarchical name scheme used for routing
- [Sessions](../../architecture/sessions/index.md) — The session layer that applications use to communicate over the Data Plane
- [SLIM Controller](../controller/index.md) — The control plane component that manages Data Plane nodes
