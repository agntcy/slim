# Integrations

SLIM is the secure transport layer for agent communication. Beyond its native
messaging APIs, SLIM integrates with common agent and observability protocols so
you can run existing tooling over the same encrypted overlay.

- [SLIMRPC](./slim-rpc.md): Protobuf RPC over SLIM, analogous to gRPC but
  with SLIM's end-to-end encryption and topology-independent routing.
- [SLIM A2A](./slim-a2a.md): Run the Agent-to-Agent (A2A) protocol over
  SLIM for secure, low-latency agent-to-agent communication.
- [MCP over SLIM](./slim-mcp.md): Connect Model Context Protocol (MCP)
  clients and servers through SLIM instead of direct HTTP.
- [OpenTelemetry over SLIM](./slim-otel.md): Export traces and metrics
  from SLIM nodes and agents using OpenTelemetry.
