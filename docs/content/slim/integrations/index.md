---
icon: material/puzzle
---

# Integrations

SLIM is the secure transport layer for agent communication. Beyond its native messaging APIs, SLIM integrates with common agent and observability protocols so you can run existing tooling over the same encrypted overlay.

<div class="grid cards" markdown>

-   :material-api:{ .lg .middle } **SLIMRPC**

    ---

    Protobuf RPC over SLIM, analogous to gRPC with end-to-end encryption and topology-independent routing.

    [:octicons-arrow-right-24: Learn more](./slimrpc/index.md)

-   :material-robot:{ .lg .middle } **SLIM A2A**

    ---

    Run the Agent-to-Agent (A2A) protocol over SLIM for secure, low-latency agent communication.

    [:octicons-arrow-right-24: Learn more](./a2a.md)

-   :material-connection:{ .lg .middle } **MCP over SLIM**

    ---

    Connect Model Context Protocol (MCP) clients and servers through SLIM instead of direct HTTP.

    [:octicons-arrow-right-24: Learn more](./mcp.md)

-   :material-chart-timeline-variant:{ .lg .middle } **OpenTelemetry over SLIM**

    ---

    Export traces and metrics from SLIM nodes and agents using OpenTelemetry.

    [:octicons-arrow-right-24: Learn more](./otel.md)

</div>
