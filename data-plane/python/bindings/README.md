# SLIM Python Bindings

High-level asynchronous Python bindings for the SLIM data‑plane service (Rust core).
They let you embed SLIM directly into your Python application to:

- Instantiate a local SLIM service
- Run a server listener (start / stop a SLIM endpoint)
- Establish outbound client connections (`connect` / `disconnect`)
- Create, accept, configure, and delete sessions (Point2Point / Group)
- Publish / receive messages (point‑to‑point or group (channel) based)
- Manage routing and subscriptions (add / remove routes, subscribe / unsubscribe)
- Configure identity & trust (shared secret, static JWT, dynamic signing JWT, JWKS auto‑resolve, Spire)
- Integrate tracing / OpenTelemetry

---

## Supported Session Types

| Type        | Description                                                                              | Sticky Peer | Metadata | MLS (group security) |
|-------------|------------------------------------------------------------------------------------------|-------------|----------|----------------------|
| Point2Point | Point-to-point with a fixed destination                                                  | Yes         | Yes      | Yes                  |
| Group       | Many-to-many via channel/topic name (channel moderator can invite/remove participants)   | N/A         | Yes      | Yes                  |

---

## Identity & Authentication

SLIM supports pluggable identity strategies for both the provider (how a local identity token/credential is produced) and the verifier (how inbound peer credentials are validated).

### Provider Variants

| Variant                             | Use Case / Scenario                          | Notes |
|------------------------------------|-----------------------------------------------|-------|
| `IdentityProvider.SharedSecret`    | HMAC-based token (shared symmetric key)       | Full HMAC (nonce+timestamp, optional replay cache); validity window & clock skew; rotate by replacing secret |
| `IdentityProvider.StaticJwt`       | Pre-issued token from file (ops managed)      | Simple; no signing key in process; rotate by replacing file |
| `IdentityProvider.Jwt`             | Dynamically signed JWT                        | Supports `exp`, optional `iss`, `aud[]`, `sub`; controllable `duration` |
| `IdentityProvider.Spire`           | SPIRE / SPIFFE based workload identity        | Obtains SVID/JWT via SPIRE Workload API; offloads trust & rotation |

### Verifier Variants

| Variant                             | Use Case / Scenario                               | Notes |
|------------------------------------|----------------------------------------------------|-------|
| `IdentityVerifier.SharedSecret`    | HMAC verification of shared-secret tokens          | Verifies MAC, nonce, timestamp; optional replay cache; supports custom claims (no iss/aud/sub) |
| `IdentityVerifier.Jwt`             | Standard JWT / OIDC verification                   | Public key OR JWKS auto-resolve; optional strict claim requirements |
| `IdentityVerifier.Spire`           | Verify SPIRE-issued identities / tokens            | Uses SPIRE Workload API; matches SPIFFE IDs and audiences |

### JWT Algorithms

Supported signing / verification algorithms:

| Symmetric | RSA      | PS (RSA-PSS) | EC      | Other  |
|-----------|----------|--------------|---------|--------|
| HS256     | RS256    | PS256        | ES256   | EdDSA  |
| HS384     | RS384    | PS384        | ES384   |        |
| HS512     | RS512    | PS512        |         |        |

(Select via `Algorithm.<Variant>` when constructing a `Key`.)

### Key Formats

| Format  | Description                                  |
|---------|----------------------------------------------|
| `KeyFormat.Pem`  | PEM encoded key material (file or string) |
| `KeyFormat.Jwk`  | Single JSON Web Key                  |
| `KeyFormat.Jwks` | JWKS (set of keys)                   |

Key data can be provided either by file path (`KeyData.File(path="path.pem")`) or in-memory content (`KeyData.Content(content=pem_string)`).

### Provider Examples

Dynamic signing JWT:

```python
import datetime
from slim_bindings import IdentityProvider, Key, Algorithm, KeyFormat, KeyData

private_key = Key(Algorithm.RS256, KeyFormat.Pem, KeyData.File(path="private_key.pem"))
provider = IdentityProvider.Jwt(
    private_key=private_key,
    duration=datetime.timedelta(minutes=30),
    issuer="my-issuer",
    audience=["peer-service"],
    subject="local-service",
)
```

Static pre-issued token (already on disk):

```python
from slim_bindings import IdentityProvider
provider = IdentityProvider.StaticJwt(path="issued.jwt")
```

SPIRE provider (automatic SVID / JWT retrieval):

```python
from slim_bindings import IdentityProvider
provider = IdentityProvider.Spire(
    socket_path="/tmp/spire-agent/public/api.sock",     # optional; default search paths if None
    target_spiffe_id="spiffe://example.org/my-service", # optional filter
    jwt_audiences=["peer-service"]                      # optional requested audiences
)
```

### Verifier Examples

Strict JWT verification (public key):

```python
from slim_bindings import IdentityVerifier, Key, Algorithm, KeyFormat, KeyData

pub_key = Key(Algorithm.RS256, KeyFormat.Pem, KeyData.File(path="public_key.pem"))
verifier = IdentityVerifier.Jwt(
    public_key=pub_key,
    autoresolve=False,
    issuer="my-issuer",
    audience=["peer-service"],
    subject="local-service",
    require_iss=True,
    require_aud=True,
    require_sub=True,
)
```

JWKS auto‑resolve (no static key needed — discovery used):

```python
verifier = IdentityVerifier.Jwt(
    public_key=None,          # trigger auto-resolution
    autoresolve=True,
    issuer="https://issuer.example.com",
    audience=["peer-service"],
    require_iss=True,
    require_aud=True,
)
```

SPIRE verifier:

```python
verifier = IdentityVerifier.Spire(
    socket_path="/tmp/spire-agent/public/api.sock",
    target_spiffe_id="spiffe://example.org/peer-service",
    jwt_audiences=["peer-service"]
)
```

### JWKS Auto‑Resolution Workflow

When `IdentityVerifier.Jwt` is created with `autoresolve=True` and no explicit `public_key`:

1. Perform OpenID Provider Discovery (`/.well-known/openid-configuration`) to find `jwks_uri`.
2. If discovery fails, attempt `/.well-known/jwks.json` directly.
3. Fetch & cache key set (with TTL); prefer matching `kid`, else fall back to compatible algorithm.
4. Periodically refresh before expiry (background).

### Choosing a Strategy

| Environment        | Recommended Provider / Verifier Pair                         |
|--------------------|---------------------------------------------------------------|
| Local Dev / Tests  | SharedSecret / SharedSecret                                   |
| Simple Staging     | StaticJwt / Jwt (public key)                                  |
| Production         | Jwt (dynamic signing) / Jwt (public key or JWKS)              |
| Zero-Trust / SPIFFE| Spire / Spire                                                 |

Use strict claim requirements (`require_*`) in production to avoid accepting tokens missing critical identity attributes.


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
    slim = slim_bindings.Slim(local_name, provider, verifier)

    # 2. (Optionally) connect as a client to a remote endpoint
    # await slim.connect({"endpoint": "http://127.0.0.1:50000", "tls": {"insecure": True}})

    # 4. Wait for inbound session
    print("Waiting for an inbound session...")
    session = await slim.listen_for_session()

    # Wait for messages and reply
    try:
        while True:
            msg_ctx, payload = await session.get_message()
            print("Received:", payload)
            handle = await session.publish(b"echo:" + payload)
            
            # Wait for message to be delivered end-to-end
            await handle
    except Exception as e:
        print("Error:", e)
        pass

asyncio.run(main())
```

### 3. Outbound Session (PointToPoint)

```python
remote = slim_bindings.Name("org", "namespace", "peer")
await slim.set_route(remote)
session = await slim.create_session(
    dest_name=remote,
    session_config=slim_bindings.SessionConfiguration.PointToPoint(
        max_retries=5,
        timeout=datetime.timedelta(seconds=5),
        mls_enabled=True,
        metadata={"trace_id": "abc123"},
    )
)

handle = await session.publish(b"hello")
await handle

ctx, reply = await session.get_message()
print("Reply:", reply)

# Delete session when done. This will also end the session at the remote peer.
handle = await slim.delete_session(session)
await handle
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
    "slim-bindings~=0.7.0"
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
slim-bindings = "~=0.7.0"
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

https://github.com/agntcy/slim/tree/slim-v0.7.0/data-plane/python/bindings/examples

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
