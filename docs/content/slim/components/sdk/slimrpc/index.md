# SLIMRPC

SLIMRPC is Protocol Buffers RPC over SLIM. It is analogous to gRPC (which runs over HTTP/2), but uses the SLIM session layer as the transport. You define services in `.proto` files, generate language bindings with the [SLIMRPC compiler](./compiler.md), and interact with the generated stubs much like standard gRPC — while gaining SLIM's built-in security, reliable delivery, and end-to-end encryption.

## Why SLIMRPC?

| | gRPC | SLIMRPC |
|---|---|---|
| **Transport** | HTTP/2 | SLIM sessions |
| **Service discovery** | External (DNS, load balancer) | Built into SLIM |
| **End-to-end encryption** | TLS (channel only) | MLS (application layer) |
| **Multi-cloud / multi-cluster** | Requires ingress / API gateway | Native SLIM routing |
| **Streaming patterns** | Unary, server-stream, client-stream, bidi | Same |

## SLIM Naming

The server subscribes to a single SLIM name — its base identity. The service name and method are not encoded in the SLIM name; they travel as message metadata on each call.

For example, a server with SLIM name `myorg/default/my-service` handles every method of every service registered on it. The `service` and `method` metadata fields tell the server which handler to invoke:

| Metadata key | Example value |
|---|---|
| `service` | `example_service.Test` |
| `method` | `ExampleUnaryUnary` |
| `rpc-id` | `d3f1a2b4-...` (UUID-v4) |
| `slimrpc-dir` | `req` |

## How It Works

### Single Session, Many Calls

The SLIMRPC channel maintains **one persistent SLIM session** to the remote server. All concurrent RPC calls share that session — there is no per-method session negotiation.

```
Client                               Server
  │                                     │
  │── session establishment ───────────►│
  │                                     │
  │── [rpc-id=A, service=…, method=…] ─►│  ← first message of call A
  │── [rpc-id=B, service=…, method=…] ─►│  ← first message of call B (concurrent)
  │◄─ [rpc-id=A] ───────────────────────│  ← response to call A
  │── [rpc-id=A] ──────────────────────►│  ← streaming continuation of call A
  │◄─ [rpc-id=B] ───────────────────────│  ← response to call B
```

### RPC ID and Multiplexing

Every call gets a unique **RPC ID** (UUID-v4) included in the first message. Subsequent messages in a streaming call carry only the `rpc-id` — the service and method are not repeated.

On the server, a per-session demultiplexer:

1. Reads each message from the shared session
2. Extracts the `rpc-id`
3. If a handler exists for that `rpc-id` (an in-progress streaming call), forwards the message to it
4. If no handler exists, reads `service` and `method` to dispatch a new handler task

This allows many concurrent RPC calls — unary or streaming — to share a single SLIM session with no head-of-line blocking between calls.

### Lazy Session Creation

The channel creates the SLIM session on the first RPC call and reuses it for all subsequent calls. If the session dies, the channel recreates it transparently on the next call.

## Multicast RPC

A group channel fans out a single RPC call to multiple servers simultaneously. Create the channel with `Channel.new_group_with_connection` (or the language-specific equivalent), passing a list of server names. Responses arrive as an async stream of `(context, response)` pairs, one per server.

This is useful for broadcasting configuration, querying multiple services, or scatter-gather patterns.

## Supported Languages

SLIMRPC code generation is supported for Python, Go, Java, Kotlin, and .NET (C#). See the [Compiler](./compiler.md) page for installation and usage.

## Tutorials

- [Serving a SLIMRPC Server](../tutorials/slimrpc/tutorial-serve.md) — define a proto, generate stubs, implement handlers, and serve
- [Using a SLIMRPC Server](../tutorials/slimrpc/tutorial-client.md) — create a channel and call all four streaming patterns
- [Multicast SLIMRPC](../tutorials/slimrpc/tutorial-multicast.md) — fan out a single call to multiple servers and collect their responses
