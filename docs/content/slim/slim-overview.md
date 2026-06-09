# Overview

Secure Low-Latency Interactive Messaging (SLIM) is a secure, scalable, and developer-friendly messaging framework that
provides the transport layer for agent communication protocols like A2A. While
A2A defines *what* agents say (message formats, task semantics, coordination
patterns), SLIM defines *how* these messages are securely delivered across
distributed networks.

At its core, SLIM combines:

- **gRPC's performance and reliability** — Built on HTTP/2 for efficient,
  multiplexed transport
- **Messaging capabilities** — Native support for channels and group
  communication
- **End-to-end encryption** — Using Message Layer Security (MLS) protocol
- **Native RPC support** — SRPC (SLIM RPC) for request-response patterns
  alongside messaging
- **Distributed architecture** — Separate control and data planes for
  scalability and management
- **Protocol flexibility** — Transport layer for A2A, MCP, and custom agent
  protocols

SLIM enables AI agents to communicate securely whether they're running in a data
center, in a browser, on mobile devices, or across organizational boundaries —
all while maintaining low latencies and strong security guarantees.

## SLIM Components

SLIM is composed of two main components that work together to provide secure,
scalable messaging infrastructure:

- [SLIM Messaging Layer](./slim-data-plane.md): The data plane component
  that handles message routing, delivery, and secure communication between
  applications. It consists of two layers: the session layer that provides
  end-to-end encryption (using the MLS protocol) and reliable message delivery,
  and the data plane that enables efficient message distribution across the
  network.

- [SLIM Controller](./slim-controller.md): The control plane component that
  manages SLIM node configurations, monitors the network, and provides a unified
  interface for administering the messaging infrastructure. It enables
  centralized management of routes, connections, and node deployments.

### Architecture Overview

The following diagram illustrates how SLIM components are distributed across
applications and intermediate routing nodes:

```mermaid
graph TB
    subgraph "Application Node (Agent A)"
        A1[Session Layer 1]
        A2[Session Layer 2]
        A3[Data Plane Client]
        A1 --> A3
        A2 --> A3
    end
    
    subgraph "Intermediate SLIM Node 1"
        I1[Data Plane]
    end
    
    subgraph "Intermediate SLIM Node 2"
        I2[Data Plane]
    end
    
    subgraph "Application Node (Agent B)"
        B1[Session Layer 1]
        B2[Session Layer 2]
        B3[Data Plane Client]
        B1 --> B3
        B2 --> B3
    end
    
    subgraph "Control Plane"
        CP[Configuration & Monitoring]
    end
    
    A3 -.->|encrypted messages| I1
    I1 -.->|route| I2
    I2 -.->|encrypted messages| B3
    
    CP -.->|manages| I1
    CP -.->|manages| I2
    
    style A1 fill:#4a90e2,stroke:#2e5c8a,stroke-width:2px,color:#fff
    style A2 fill:#4a90e2,stroke:#2e5c8a,stroke-width:2px,color:#fff
    style A3 fill:#f39c12,stroke:#d68910,stroke-width:2px,color:#fff
    style B1 fill:#4a90e2,stroke:#2e5c8a,stroke-width:2px,color:#fff
    style B2 fill:#4a90e2,stroke:#2e5c8a,stroke-width:2px,color:#fff
    style B3 fill:#f39c12,stroke:#d68910,stroke-width:2px,color:#fff
    style I1 fill:#f39c12,stroke:#d68910,stroke-width:2px,color:#fff
    style I2 fill:#f39c12,stroke:#d68910,stroke-width:2px,color:#fff
    style CP fill:#7f8c8d,stroke:#5a6970,stroke-width:2px,color:#fff
```

### Component Distribution

SLIM's architecture enables efficient distribution of components:

### Pure Data Plane

The `slim` binary and Docker images are distributed as pure
data-plane artifacts. Since SLIM routing nodes only forward messages and don't
participate in application sessions, they don't need the session layer. This
keeps the infrastructure lightweight, fast, and simple to deploy.

### Language Bindings

Libraries (Python, Go, etc.) include both the data plane
client and the session layer on top. Applications use these bindings to get
the full stack: secure, reliable, encrypted communication with automatic session
management.

### Separation of Concerns

You can run a global network of SLIM routing nodes
without any application logic, while your agents use the rich, full-featured
language bindings for their communication needs. The control plane manages the
routing infrastructure independently, ensuring the network operates efficiently.

To get started with SLIM, see the [Getting Started with
SLIM](../slim/slim-howto.md) guide.
