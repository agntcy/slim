# SLIM Python Bindings

High-level asynchronous Python bindings for the SLIM data‑plane service (Rust core).
They let you embed SLIM directly into your Python application to:

- Instantiate a local SLIM service (`Slim.new`)
- Run a server listener (start / stop a SLIM endpoint)
- Establish outbound client connections (`connect` / `disconnect`)
- Create, accept, configure, and delete sessions (Point2Point / Group)
- Publish / receive messages (point‑to‑point or group (channel) based)
- Manage routing and subscriptions (add / remove routes, subscribe / unsubscribe)
- Configure identity & trust (shared secret, static JWT, dynamic signing JWT, JWKS auto‑resolve)
- Integrate tracing / OpenTelemetry

---

## Supported Session Types

| Type        | Description                                                                              | Sticky Peer | Metadata | MLS (group security) |
|-------------|------------------------------------------------------------------------------------------|-------------|----------|----------------------|
| Point2Point | Point-to-point with a fixed destination                                                  | Yes         | Yes      | Yes                  |
| Group       | Many-to-many via channel/topic name (channel moderator can invite/remove participants)   | N/A         | Yes      | Yes                  |

---

## Identity & Authentication

You can choose among multiple identity provider / verifier strategies:

| Provider Variant                      | Use Case                               | Notes |
|--------------------------------------|-----------------------------------------|-------|
| `IdentityProvider.SharedSecret`    | Local dev / tests                       | Symmetric; not for production |
| `IdentityProvider.StaticJwt`       | Pre-issued token loaded from file       | No key rotation; simple |
| `IdentityProvider.Jwt`             | Dynamically signed JWT (private key)    | Supports exp, iss, aud, sub, duration |
| `IdentityVerifier.Jwt`             | Verifies JWT (public key or JWKS auto)  | Optional claim requirements (`require_iss`, etc.) |
| `IdentityVerifier.SharedSecret`    | Matches shared secret provider          | Symmetric validation |

JWKS auto‑resolution (when configured in the verifier with `autoresolve=True`) will:
1. Try OpenID discovery (`/.well-known/openid-configuration`) for `jwks_uri`
2. Fallback to `/.well-known/jwks.json`
3. Cache the key set with a TTL and prefer `kid` match, else algorithm match.

---

## Quick Start

### 1. Install

```bash
pip install slim-bindings
```

### 2. Minimal Receiver Example

```python
import asyncio
import slim_bindings

async def main():
    # 1. Create identity
    provider = slim_bindings.IdentityProvider.SharedSecret(identity="demo", shared_secret="secret")
    verifier = slim_bindings.IdentityVerifier.SharedSecret(identity="demo", shared_secret="secret")

    local_name = slim_bindings.Name("org", "namespace", "demo")
    slim = await slim_bindings.Slim.new(local_name, provider, verifier)

    # 2. (Optionally) connect as a client to a remote endpoint
    # await slim.connect({"endpoint": "http://127.0.0.1:50000", "tls": {"insecure": True}})

    # 3. (Optionally) run a local server (insecure TLS for local dev)
    # await slim.run_server({"endpoint": "127.0.0.1:40000", "tls": {"insecure": True}})

    # 4. Wait for inbound session
    print("Waiting for an inbound session...")
    session = await slim.listen_for_session()

    # 5. Receive one message and reply
    msg_ctx, payload = await session.get_message()
    print("Received:", payload)
    await session.publish_to(msg_ctx, b"echo:" + payload)

    # 6. Clean shutdown
    await slim.delete_session(session)
    await slim.stop_server("127.0.0.1:40000")

asyncio.run(main())
```

### 3. Outbound Session (PointToPoint)

```python
remote = slim_bindings.Name("org", "namespace", "peer")
session = await slim.create_session(
    slim_bindings.SessionConfiguration.PointToPoint(
        peer_name=remote,
        mls_enabled=True,
        metadata={"trace_id": "abc123"},
    )
)
await slim.set_route(remote)
await session.publish(b"hello")
ctx, reply = await session.get_message()
print("Reply:", reply)
await slim.delete_session(session)
```

---

## Tracing / Observability

Initialize tracing (optionally enabling OpenTelemetry export):

```python
await slim_bindings.init_tracing({
    "log_level": "info",
    "opentelemetry": {
        "enabled": True,
        "grpc": {"endpoint": "http://localhost:4317"}
    }
})
```

---

## Installation

```bash
pip install slim-bindings
```

---

## Include as Dependency

### With `pyproject.toml`

```toml
[project]
name = "slim-example"
version = "0.1.0"
description = "Python program using SLIM"
requires-python = ">=3.10"
dependencies = [
    "slim-bindings>=0.6.0"
]
```

### With Poetry

```toml
[tool.poetry]
name = "slim-example"
version = "0.1.0"
description = "Python program using SLIM"

[tool.poetry.dependencies]
python = ">=3.10,<3.13"
slim-bindings = ">=0.5.0"
```

---

## Feature Highlights

| Area            | Capability |
|-----------------|------------|
| Server          | `run_server`, `stop_server` |
| Client          | `connect`, `disconnect`, automatic subscribe to local name |
| Routing         | `set_route`, `remove_route` |
| Subscriptions   | `subscribe`, `unsubscribe` |
| Sessions        | `create_session`, `listen_for_session`, `delete_session`, `set_session_config` |
| Messaging       | `publish`, `publish_to`, `get_message` |
| Identity        | Shared secret, static JWT, dynamic JWT signing, JWT verification (public key / JWKS) |
| Tracing         | Structured logs & optional OpenTelemetry export |

---

## Example Programs

Complete runnable examples (point2point, group, server) live in the repository:

https://github.com/agntcy/slim/tree/slim-v0.5.0/data-plane/python/bindings/examples

You can install and invoke them (after building) via:

```bash
slim-bindings-examples point2point ...
slim-bindings-examples group ...
slim-bindings-examples slim ...
```

---

## When to Use Each Session Type

| Use Case                          | Recommended Type |
|----------------------------------|-------------------|
| Stable peer workflow / stateful  | Point2Point       |
| Group chat / fan-out             | Group             |

---

## Security Notes

- Prefer asymmetric JWT-based identity in production.
- Rotate keys periodically and enable `require_iss`, `require_aud`, `require_sub`.
- Shared secret is only suitable for local tests and prototypes.

---

## License

Apache-2.0 (see repository for full license text).
