# Routing

SLIM uses a domain-based routing model managed by the SLIM Controller. The Controller maintains the desired connectivity between domains of SLIM nodes and propagates routing information to the data plane using a declarative reconciliation loop. This page explains the three core concepts — **domains**, **links**, and **segments** — and how they combine to form flexible network topologies.

## Domains

A **domain** is a set of SLIM data plane nodes that share a common identity — typically all nodes within a single deployment or Kubernetes cluster. Each node belongs to exactly one domain, declared in its configuration:

```yaml
services:
  slim/0:
    domain_name: "cluster-a.example"
```

**Intra-domain connectivity** is handled automatically by the data plane: nodes in the same domain discover each other via peer discovery and route messages between themselves without Controller involvement.

**Inter-domain connectivity** is managed by the Controller. When nodes from different domains register, the Controller creates links between them and installs route subscriptions so messages can flow across domain boundaries.

Within each domain, one node is selected at random as the **gateway** — the node that holds the inter-domain link and forwards traffic to and from other domains. If the gateway node crashes or deregisters, the Controller performs **gateway failover**: it reassigns the inter-domain link to a sibling node in the same domain, maintaining connectivity without operator intervention.

## Links

A **link** is a directed gRPC connection between two nodes in different domains. The source node initiates the connection to the destination node's `external_endpoint`. Links are created and managed entirely by the Controller — application code and data plane nodes do not create links directly.

```mermaid
graph LR
    subgraph "Domain A"
        gw-a["Node A\n(gateway)"]
    end
    subgraph "Domain B"
        gw-b["Node B\n(gateway)"]
    end
    gw-a -- "link (gRPC)" --> gw-b
```

### Link Lifecycle

Links pass through the following states:

```mermaid
stateDiagram-v2
    [*] --> Pending
    Pending --> Connecting: connection sent to node
    Connecting --> Applied: node establishes connection
    Connecting --> Failed: connection failed
    Applied --> Pending: connection lost or endpoint changed
    Pending --> Deleted: marked for removal
    Deleted --> [*]: deletion confirmed
```

Once a link reaches `Applied`, the Controller installs route subscriptions over it so messages for remote names are forwarded through the link automatically.

### Inspecting Links

```bash
# List all inter-domain links
slimctl controller link list

# List routes (subscriptions installed over links)
slimctl controller route list

# List nodes and their domain assignment
slimctl controller node list
```

## Topology

The **topology** configuration in the Controller defines which domains are allowed to form inter-domain links. It is expressed as an adjacency list: each entry declares a domain name and the domains it connects to. All links are bidirectional — if domain A lists domain B as a neighbour, the link between them is established in both directions.

The wildcard `"*"` matches all registered domains and is resolved at runtime when new nodes register.

### Full Mesh (Default)

If no topology is configured, the Controller defaults to full mesh: every domain is linked to every other domain.

```yaml
topology: {}
```

Use full mesh when all deployments need to communicate with each other and there is no need to restrict routing.

### Star Topology

A hub domain connects to all others; spoke domains can only reach each other by routing through the hub.

```yaml
topology:
  links:
    - name: cloud
      neighbors: ["*"]
```

Use a star topology when you have a central service (e.g. a cloud-hosted coordination layer) that all edge deployments connect to, but edge deployments should not connect directly to each other.

### Explicit Pairs

Only specific domain pairs are allowed to form links:

```yaml
topology:
  links:
    - name: cloud
      neighbors: [customer-a, customer-b]
    - name: customer-a
      neighbors: [cloud]
    - name: customer-b
      neighbors: [cloud]
```

### Chain Topology

Domains form a linear chain; multi-hop routing via the Shortest Path Tree algorithm handles transit automatically:

```yaml
topology:
  links:
    - name: domain-a
      neighbors: [domain-b]
    - name: domain-b
      neighbors: [domain-a, domain-c]
    - name: domain-c
      neighbors: [domain-b, domain-d]
    - name: domain-d
      neighbors: [domain-c]
```

## Segments

**Segments** partition the network into independent routing domains. Nodes in one segment are completely invisible to nodes in other segments — routes are only expanded within a segment's topology graph, and no links are created between domains that do not share an edge in any segment.

Segments are used to enforce **multi-tenant isolation**: different customers or deployments can share the same SLIM infrastructure while being unable to route messages to each other.

When segments are defined, the top-level `topology.links` configuration is ignored — segments fully control both link creation and route expansion.

### Named Segments

Explicit segments for multi-tenant isolation:

```yaml
topology:
  segments:
    - name: customer-1
      links:
        - name: cloud
          neighbors: [cluster-a]
    - name: customer-2
      links:
        - name: cloud
          neighbors: [cluster-b, cluster-c]
```

In this example, `cluster-a` can route to `cloud` (and vice versa), but `cluster-a` cannot reach `cluster-b` or `cluster-c` at all — they are in separate routing domains.

### Template Segments with `$domain`

The special token `$domain` causes a segment definition to be **instantiated once per registered domain**. This enables dynamic per-tenant isolation without manually listing every domain:

```yaml
topology:
  segments:
    - name: segment-$domain
      links:
        - name: cloud
          neighbors: [$domain]
```

When a node from `customer-a` registers, the Controller instantiates a segment named `segment-customer-a` with links `cloud <-> customer-a`. When a node from `customer-b` registers, another segment `segment-customer-b` is instantiated with `cloud <-> customer-b`. Because each customer domain exists in its own segment, `customer-a` and `customer-b` cannot route to each other even though they both connect to `cloud`.

### Inspecting Segments

```bash
# List all segments and their domain membership
slimctl controller segment list
```

The output shows each segment's name, the domains it contains, and the edges (links) in its adjacency graph.

## Shortest Path Tree Routing

When a SLIM application subscribes to a name, the Controller installs route subscriptions on data plane nodes using a **Shortest Path Tree (SPT)** algorithm. The SPT computes a loop-free forwarding tree rooted at the first domain that announced the name:

- **Upward routes**: installed on non-root domain gateways, pointing toward the root — used to deliver messages from any domain to the first announcer
- **Downward routes**: installed when additional domains announce the same name, pointing away from the root toward the new announcers — used to fan out messages to all subscribers

This ensures that multi-hop routing (e.g. spoke-a → hub → spoke-b in a star topology) works correctly without creating forwarding loops, even across non-directly-connected domains.

## Choosing a Topology

| Topology | When to use |
|----------|-------------|
| Full mesh | All domains communicate freely; simple deployments |
| Star (hub + `"*"`) | Hub-and-spoke; edge deployments connect via a central service |
| Explicit pairs | Controlled access; specific domains should reach specific others |
| Chain | Linear pipelines; multi-hop routing handled automatically |
| Segments | Multi-tenant isolation; customers must not route to each other |
| `$domain` template | Dynamic per-tenant segments; domains register without pre-configuration |

## Related

- [SLIM Controller Overview](../components/controller/index.md) — The component that manages domains, links, and segments
- [Controller Configuration Reference](../components/controller/config.md) — Full topology configuration options
- [Naming](./naming.md) — How client and channel names work in SLIM
- [Groups](./sessions/group.md) — Group sessions for multi-agent communication (distinct from routing domains)
