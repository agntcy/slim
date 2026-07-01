# SLIM Controller

The [SLIM](../../../index.md) Controller is a central management component that
orchestrates and manages SLIM nodes in a distributed messaging system. It
provides a unified interface for configuring routes, managing node registration,
and coordinating communication between nodes.

The Controller serves as the central coordination point for SLIM infrastructure,
offering both northbound and southbound interfaces. The northbound interface
allows external systems and administrators to configure and manage the SLIM
network. The southbound interface enables SLIM nodes to register and receive
configuration updates.

## Key Features

- **Centralized Node Management**: Register and manage multiple SLIM nodes from a single control point.
- **Route Configuration**: Set up message routing between nodes through the Controller.
- **Bidirectional Communication**: Supports both northbound and southbound gRPC interfaces.
- **Connection Orchestration**: Manages connections and subscriptions between SLIM nodes.

## Architecture

The Controller implements northbound and southbound gRPC interfaces.

The northbound interface provides management capabilities for external systems
and administrators, such as [slimctl](../cli/install.md). It includes:

- **Route Management**: Create, list, and manage message routes between nodes.
- **Connection Management**: Set up and monitor connections between SLIM nodes.
- **Node Discovery**: List registered nodes and their status.

The southbound interface allows SLIM nodes to register with the Controller and
receive configuration updates. It includes:

- **Node Registration**: Nodes can register themselves with the Controller.
- **Node De-registration**: Nodes can unregister when shutting down.
- **Configuration Distribution**: The Controller can push configuration updates to registered nodes.
- **Bidirectional Communication**: Supports real-time communication between the Controller and nodes.

### Control Plane Architecture

```mermaid
graph TB
    %% User and CLI
    User[👤 User/Administrator]
    CLI[slimctl CLI Tool]

    %% Control Plane Components
    subgraph "Control Plane"
        Controller[SLIM Controller<br/>- Northbound API<br/>- Southbound API<br/>- Node Registry]
        Config[Configuration<br/>Store]
    end

    %% Data Plane Nodes
    subgraph "Data Plane"
        Node1[SLIM Node 1<br/>- Message Routing<br/>- Client Connections]
        Node2[SLIM Node 2<br/>- Message Routing<br/>- Client Connections]
        Node3[SLIM Node 3<br/>- Message Routing<br/>- Client Connections]
    end

    %% Client Applications
    subgraph "Applications"
        App1[Client App 1]
        App2[Client App 2]
        App3[Client App 3]
    end

    %% User interactions
    User -->|Commands| CLI
    CLI -->|gRPC Northbound<br/>Port 50051| Controller

    %% Control plane interactions
    Controller <-->|Store/Retrieve<br/>Configuration| Config

    %% Southbound connections
    Controller <-->|gRPC Southbound<br/>Port 50052<br/>Registration & Config| Node1
    Controller <-->|gRPC Southbound<br/>Port 50052<br/>Registration & Config| Node2
    Controller <-->|gRPC Southbound<br/>Port 50052<br/>Registration & Config| Node3

    %% Inter-node communication
    Node1 <-->|Message Routing| Node2
    Node2 <-->|Message Routing| Node3
    Node1 <-->|Message Routing| Node3

    %% Application connections
    App1 -->|SLIM Protocol| Node1
    App2 -->|SLIM Protocol| Node2
    App3 -->|SLIM Protocol| Node3

    %% Styling for light/dark mode compatibility
    classDef user fill:#4A90E2,stroke:#2E5D8A,stroke-width:2px,color:#FFFFFF
    classDef control fill:#9B59B6,stroke:#6A3A7C,stroke-width:2px,color:#FFFFFF
    classDef data fill:#27AE60,stroke:#1E8449,stroke-width:2px,color:#FFFFFF
    classDef app fill:#F39C12,stroke:#D68910,stroke-width:2px,color:#FFFFFF

    class User,CLI user
    class Controller,Config control
    class Node1,Node2,Node3 data
    class App1,App2,App3 app
```

### Control Flow Sequence

```mermaid
sequenceDiagram
    participant User
    participant CLI as slimctl CLI
    participant Controller as SLIM Controller
    participant Node as SLIM Node
    participant App as Client App

    %% Node Registration
    Note over Node,Controller: Node Startup & Registration
    Node->>Controller: Register Node (Southbound)
    Controller->>Node: Registration Ack
    Controller->>Controller: Store Node Info

    %% Route Management via CLI
    Note over User,Controller: Route Management
    User->>CLI: slimctl route add org/ns/agent via config.json
    CLI->>Controller: CreateRoute Request (Northbound)
    Controller->>Controller: Validate & Store Route
    Controller->>Node: Push Route Configuration (Southbound)
    Node->>Controller: Configuration Ack
    Controller->>CLI: Route Created Response
    CLI->>User: Success Message

    %% Connection Management
    Note over User,Controller: Connection Management
    User->>CLI: slimctl connection list --node-id=slim/1
    CLI->>Controller: ListConnections Request (Northbound)
    Controller->>Controller: Retrieve Connection Info
    Controller->>CLI: Connections List Response
    CLI->>User: Display Connections

    %% Application Communication
    Note over App,Node: Application Messaging
    App->>Node: Connect & Subscribe
    Node->>App: Connection Established
    App->>Node: Publish Message
    Node->>Node: Route Message via Controller Config

    %% Node Status Updates
    Note over Node,Controller: Status Monitoring
    Node->>Controller: Status Update (Southbound)
    Controller->>Controller: Update Node Status
```

## Managing Nodes

Nodes can register themselves with the Controller upon startup. Once registered, the Controller can communicate with nodes using the same connection.

To enable self-registration, configure the nodes with the Controller address:

```yaml
services:
  slim/1:
    dataplane:
      servers: []
      clients: []
    controller:
      servers: []
      clients:
        - endpoint: "http://<controller-address>:50052"
          tls:
            insecure: true
```

Routes between SLIM nodes are automatically created by the Controller upon receiving new subscriptions from clients. Nodes can also be managed manually through slimctl.

## Related

- [Installation Guide](./install.md) — Install the SLIM Controller
- [Configuration Reference](./config.md) — Full YAML configuration reference
- [SLIM CLI](../cli/install.md) — Install and configure `slimctl`
- [Command Reference](../cli/reference/index.md) — Full `slimctl` command reference
