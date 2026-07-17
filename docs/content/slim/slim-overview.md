---
icon: material/information-outline
---

# Introduction

**SLIM (Secure Low-Latency Interactive Messaging)** is the transport layer for the agentic AI era — a secure, peer-to-peer messaging framework that lets agents communicate across networks, organizations, and trust boundaries without rebuilding the same infrastructure every time.

## Why SLIM?

Modern agentic workloads share three characteristics that make communication hard:

<div class="grid cards" markdown>

-   :material-account-group:{ .lg .middle } **Collaborative**

    ---

    Agents share high volumes of context in real time and coordinate complex multi-step tasks. Communication must be bidirectional, low-latency, and support multiparty interaction patterns natively.

-   :material-shield-lock:{ .lg .middle } **Sensitive**

    ---

    Agents share sensitive data — code, plans, personal information, business logic. Network-level TLS is not enough: if a node is compromised, traffic can be read. Data must be encrypted end-to-end.

-   :material-earth:{ .lg .middle } **Distributed**

    ---

    Agents run across multiple clouds, organizations, and networks. Communication must traverse NAT, firewalls, and organizational boundaries without requiring agents to be directly internet-exposed.

</div>

### The gap in existing solutions

No single existing technology addresses all three requirements:

| System | Gap |
|--------|-----|
| **Message queues** (SQS, RabbitMQ, Kafka, NATS) | One-directional; no E2E encryption; can't cross org boundaries; no native RPC |
| **HTTP / gRPC** | No group communication; no E2E encryption; requires agents to be directly internet-exposed |

The result: developers keep re-implementing the missing pieces in every application.

### What SLIM brings

SLIM is built from the ground up for agentic workloads:

| Capability | How SLIM delivers it |
|-----------|----------------------|
| **Bidirectional, low-latency transport** | Built on gRPC over HTTP/2 — multiplexed, efficient, and NAT/firewall-friendly without any special configuration, with an optional WebSocket transport for HTTP-only paths and browser (WASM) clients |
| **End-to-end encryption** | [MLS (RFC 9420)](https://www.rfc-editor.org/rfc/rfc9420.txt) encrypts data at the session layer — even a compromised routing node cannot read message content |
| **Group communication** | Native multicast and group sessions; agents join shared channels identified by hierarchical names |
| **Cross-org federation** | SPIRE-based identity federation lets agents in different organizations communicate with verified cryptographic identities |
| **No central broker** | Peer-to-peer architecture; no single point of failure or trust |
| **Native RPC** | SLIMRPC provides Protobuf RPC over SLIM — all the capabilities of gRPC, plus multicast RPC over group channels |

## Interaction Patterns

SLIM natively supports four interaction patterns between agents:

| Pattern | Description |
|---------|-------------|
| **Point-to-Point** | One agent sends directly to another, identified by a hierarchical DID-based name |
| **Group / Multicast** | One agent broadcasts to a shared channel; all members receive the message |
| **RPC (P2P)** | Request-response between two agents — like gRPC but over SLIM's encrypted overlay |
| **RPC (Multicast)** | One client calls many servers simultaneously — fan-out requests to a group channel |

Agents are identified using a hierarchical naming scheme based on Decentralized Identifiers:

```text
organization/namespace/service/<hash-of-public-key>
```

Group channels use the same scheme, with `0xffffffff` as the last component to fan out to all joined members:

```text
organization/namespace/service/0xffffffff
```

## SLIM vs. Message Queues

| Capability | Message Queue | SLIM |
|-----------|---------------|------|
| Bidirectional communication | ❌ Needs a response topic | ✅ Native |
| End-to-end encryption | ❌ Broker can read all traffic | ✅ MLS E2E |
| Cross-org / internet exposure | ❌ Many impossible to expose | ✅ Works across organizations |
| Native RPC | ❌ Not natively supported | ✅ SLIMRPC |
| Group sessions (multi-party) | ❌ Topic fan-out only | ✅ MLS groups |
| Multi-org federation | ❌ Complex bespoke work | ✅ Built-in with SPIRE |
| Works behind NAT / firewalls | ⚠️ Depends on deployment | ✅ Registration model |

## Components

SLIM is composed of five components:

- **[Data Plane](./components/data-plane/index.md)** — high-performance message routing node that forwards messages between applications
- **[Controller](./components/controller/index.md)** — manages route tables, node registration, and group membership across clusters
- **[Channel Manager](./components/channel-manager/index.md)** — operator-managed service for creating and moderating group channels
- **[CLI (`slimctl`)](./components/cli/install.md)** — command-line tool for operating and interacting with SLIM nodes
- **[SDK](./components/sdk/index.md)** — language-native bindings (Python, Go, .NET, JavaScript) for building applications on SLIM

For a detailed breakdown of how the layers fit together, see [Architecture](./architecture/index.md).

## Next Steps

- [Getting Started](./slim-howto.md) — install all components and send your first message
- [Architecture](./architecture/index.md) — understand the four-layer SLIM stack
- [Components](./components/index.md) — explore each component in depth
- [Deployment](./deploy/index.md) — choose a deployment topology for your use case
