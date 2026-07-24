# Point-to-Point Session

A point-to-point session connects one application instance to exactly one remote instance. The session performs a **discovery phase** first: the initiator sends a request to the remote service's anycast name, and the SLIM network delivers it to one available instance. That instance replies with its full unique name, and the session is then bound to that specific endpoint for its lifetime.

This binding behaviour means a point-to-point session is stable across messages — the same remote instance handles the entire conversation. It is the right choice for request-response interactions, task delegation to a specific agent, and any pattern that requires conversation state to be held at one endpoint.

## Establishment Sequence

The diagram below shows the full establishment sequence when MLS is enabled. When MLS is disabled, the MLS setup phase is skipped and message exchange begins immediately after discovery.

```mermaid
sequenceDiagram
    autonumber

    participant App-A
    participant SLIM Node
    participant App-B/1
    participant App-B/2

    Note over App-A,App-B/2: Discovery
    App-A->>App-A: Init MLS state
    App-A->>SLIM Node: Discover agntcy/ns/App-B
    SLIM Node->>App-B/1: Discover agntcy/ns/App-B
    App-B/1->>SLIM Node: Discover Reply (agntcy/ns/App-B/1)
    SLIM Node->>App-A: Discover Reply (agntcy/ns/App-B/1)

    Note over App-A,App-B/2: Invite
    App-A->>SLIM Node: Invite agntcy/ns/App-B/1
    SLIM Node->>App-B/1: Invite agntcy/ns/App-B/1
    App-B/1->>App-B/1: Create new point-to-point Session
    App-B/1->>SLIM Node: Invite Reply (MLS key package)
    SLIM Node->>App-A: Invite Reply (MLS key package)
    App-A->>App-A: Update MLS state

    Note over App-A,App-B/2: MLS setup
    App-A->>SLIM Node: MLS Welcome agntcy/ns/App-B/1
    SLIM Node->>App-B/1: MLS Welcome agntcy/ns/App-B/1
    App-B/1->>App-B/1: Init MLS state
    App-B/1->>SLIM Node: Ack(MLS Welcome)
    SLIM Node->>App-A: Ack(MLS Welcome)

    Note over App-A,App-B/2: Message exchange
    App-A->>SLIM Node: Message to agntcy/ns/App-B/1
    SLIM Node->>App-B/1: Message to agntcy/ns/App-B/1
    App-B/1->>SLIM Node: Ack
    SLIM Node->>App-A: Ack
```

## Phases

**Discovery** — The initiator addresses the remote service by its three-component anycast name (`org/namespace/service`). The SLIM network delivers the discover request to one available instance, which replies with its full four-component unique name. From this point the session is bound to that specific instance.

**Invite** — The initiator sends an invite to the discovered instance's unique name. The responder creates a local session and, if MLS is enabled, returns its MLS key package.

**MLS setup** — The initiator uses the key package to generate a Welcome message, completing the MLS handshake. Both sides now share key material and the session is fully established.

**Message exchange** — Messages flow between the two bound endpoints with per-message acknowledgement (when reliability is configured).

## Related

- [Sessions](./index.md) — Session types overview, properties, and choosing between P2P and Group
- [Creating a Session](../../components/sdk/tutorials/tutorial-session.md) — SDK tutorial with P2P code examples in Python and Go
- [Naming](../naming.md) — Anycast vs. unicast addressing
