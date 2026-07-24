# MCP over SLIM

[Model Context Protocol (MCP)](https://modelcontextprotocol.io/) is an open standard for connecting AI models to tools and data sources. By default, MCP clients connect to servers over HTTP (SSE or Streamable HTTP). SLIM replaces that transport with an encrypted, identity-aware overlay — so MCP clients and servers can reach each other across networks and organizations without exposing every server to the internet.

## Why use SLIM as the MCP transport?

| | HTTP (default) | SLIM |
|---|---|---|
| **Discovery** | URL / DNS | Built into SLIM naming |
| **Encryption** | TLS (channel only) | MLS (end-to-end) |
| **Multi-cloud / multi-cluster** | Requires ingress / API gateway | Native SLIM routing |
| **Authentication** | Per-application | Shared secret, mTLS, or SPIFFE — enforced by SLIM |
| **Load balancing** | External load balancer | Native via SLIM group sessions |

## How it works

An MCP server registers under a SLIM name and calls `run_mcp_server` to listen for incoming sessions. An MCP client creates a SLIM app, resolves the server by name, and opens a session using `create_client_streams` — returning standard MCP-compatible read/write streams that the `mcp` library uses directly.

```python
# Server
name = slim_bindings.Name("org", "namespace", "my-mcp-server")
slim_app, _ = await create_local_app(name, shared_secret="secret")
await run_mcp_server(slim_app, mcp_app)

# Client
destination = slim_bindings.Name("org", "namespace", "my-mcp-server")
async with create_client_streams(client_app, destination) as (read, write):
    async with ClientSession(read, write) as session:
        await session.initialize()
        tools = await session.list_tools()
```

The SLIM session layer handles key establishment, reliable delivery, and end-to-end encryption transparently — no changes to MCP tool definitions or server logic are required.

## Implementation

| Language | Repository |
|---|---|
| Python | [agntcy/slim-mcp-python](https://github.com/agntcy/slim-mcp-python) |

## Related

- [SLIM SDK](../components/sdk/index.md) — connect and build SLIM-enabled apps
- [Sessions](../architecture/sessions/index.md) — how SLIM establishes secure sessions
- [Authentication](../architecture/authentication.md) — shared secret, mTLS, and SPIFFE options
