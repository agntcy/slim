# A2A over SLIM

[Agent-to-Agent (A2A)](https://a2a-protocol.org/) is an open protocol for inter-agent communication. It defines a layered architecture where core operations (sending tasks, streaming responses, push notifications) are specified independently of the underlying transport. Concrete transports are called **protocol bindings**.

## Standard A2A Protocol Bindings

The A2A specification defines three standard protocol bindings:

| Binding | Transport | Notes |
|---|---|---|
| **JSON-RPC** | HTTP + JSON-RPC 2.0 | Default binding; broadly compatible |
| **gRPC** | HTTP/2 + Protocol Buffers | High-performance; strong typing |
| **HTTP+JSON/REST** | HTTP + JSON | REST-style endpoints |

## SLIMRPC Protocol Binding

SLIM provides a fourth, experimental binding: **SLIMRPC**. Instead of HTTP, it uses SLIM's encrypted session layer as the transport — bringing end-to-end encryption, built-in service discovery, and native multi-cloud routing to A2A without requiring ingress controllers or API gateways.

| | HTTP bindings (default) | SLIMRPC binding |
|---|---|---|
| **Discovery** | DNS / load balancer | Built into SLIM naming |
| **Encryption** | TLS (channel only) | MLS (end-to-end, application layer) |
| **Multi-cloud / multi-cluster** | Requires ingress / API gateway | Native SLIM routing |
| **Authentication** | Per-application | JWT, mTLS, or SPIFFE — enforced by SLIM |
| **Group / multicast** | Not supported | Native via SLIM group sessions |

The wire format and protocol requirements are defined by the [A2A SLIMRPC Custom Protocol Binding](https://github.com/a2aproject/experimental-cpb-slimrpc) — an experimental specification in the A2A project. The `protocolBinding` identifier is:

```text
https://a2a-protocol.org/bindings/experimental-slimrpc/v1
```

SLIM carries A2A messages using [SLIMRPC](../components/sdk/slimrpc/index.md) — the Protocol Buffers RPC layer built on SLIM sessions. Because SLIMRPC uses the same `.proto` service definitions as gRPC, integrating A2A's `a2a.proto` is straightforward.

## Implementations

| Language | Repository |
|---|---|
| Python | [agntcy/slim-a2a-python](https://github.com/agntcy/slim-a2a-python) |
| Go | [agntcy/slim-a2a-go](https://github.com/agntcy/slim-a2a-go) |
| Java | [agntcy/slim-a2a-java](https://github.com/agntcy/slim-a2a-java) |
| .NET | [agntcy/slim-a2a-dotnet](https://github.com/agntcy/slim-a2a-dotnet) |
| Rust | [a2aproject/a2a-rs](https://github.com/a2aproject/a2a-rs/tree/main/a2a-slimrpc) |

## Related

- [A2A SLIMRPC Custom Protocol Binding spec](https://github.com/a2aproject/experimental-cpb-slimrpc) — full specification including multicast RPC
- [SLIMRPC](../components/sdk/slimrpc/index.md) — the RPC layer A2A runs over
- [Sessions](../architecture/sessions/index.md) — how SLIM establishes secure sessions between agents
- [Authentication](../architecture/authentication.md) — JWT, mTLS, and SPIFFE options
