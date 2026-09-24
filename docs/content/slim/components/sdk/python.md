# Python SDK

The SLIM Python SDK (`slim-bindings`) provides an idiomatic Python API for building applications on SLIM. Bindings are generated from the same Rust core as every other language binding via [UniFFI](https://github.com/mozilla/uniffi-rs), with async/await support through `asyncio` and native libraries bundled into platform-specific wheels.

## Requirements

| | |
|---|---|
| **Runtime** | Python 3.10 or higher |
| **Package** | [`slim-bindings`](https://pypi.org/project/slim-bindings/) on PyPI |
| **Examples** | [python/examples](https://github.com/agntcy/slim-bindings/tree/main/python/examples) in slim-bindings |

The PyPI package ships native libraries for Linux, macOS, and Windows on x64 and arm64. No additional runtime setup is required after install.

## Installation

=== "pip"

    ```bash
    pip install slim-bindings
    ```

=== "pyproject.toml"

    Add to your `pyproject.toml`:

    ```toml
    [project]
    dependencies = ["slim-bindings"]
    ```

=== "uv"

    ```bash
    uv add slim-bindings
    ```

## Getting Started

The [SDK tutorials](./tutorials/index.md) build a full application step by step — initialising the service, connecting to a node, creating an app, opening sessions, receiving messages, and adding persistence — with Python snippets shown alongside every other binding.

Start with [Connecting to SLIM](./tutorials/tutorial-connect.md).

## API Overview

| Type | Description |
|---|---|
| `slim_bindings` module | Static entry point for initialisation and global service access |
| `Service` | Manages connections and creates apps |
| `App` | Application handle for sessions, subscriptions, and routing |
| `Session` | Session for sending and receiving messages |
| `Name` | Identity in `org/namespace/app` format |
| `ReceivedMessage` | Received message with `payload` (bytes) and `context` metadata |
| `SessionConfig` | Session configuration (type, MLS, retries) |
| `ClientConfig` | Client connection configuration (endpoint, TLS, transport auth) |
| `ServerConfig` | Server listen configuration (endpoint, TLS, transport auth) |
| `OidcConfig` | OIDC transport authentication settings |
| `OidcPolicyConfig` | Claim-based access policy (`CEL`, `REGO`, `REGO_FILE`) |

UniFFI async methods require registering the running event loop with `uniffi_set_event_loop` before calling any async API.

## Transport Authentication

Separate from the app identity passed to `create_app_with_secret`, the gRPC connection to a SLIM node can carry its own credentials.

### OIDC (client credentials)

```python
import datetime

oidc = slim_bindings.OidcConfig(
    issuer_url="https://auth.example.com",
    client_id="my-client",
    client_secret="s3cr3t",
    scope="openid profile",
    timeout=datetime.timedelta(seconds=30),
)

base = slim_bindings.new_insecure_client_config("http://127.0.0.1:46357")
client_config = slim_bindings.ClientConfig(
    **{**vars(base), "auth": slim_bindings.ClientAuthenticationConfig.OIDC(config=oidc)}
)
conn_id = await service.connect_async(client_config)
```

For the refresh-token flow, set `refresh_token` (or `refresh_token_file`, which is rewritten in place as tokens rotate) instead of `client_secret`.

### OIDC (server verification)

```python
oidc = slim_bindings.OidcConfig(
    issuer_url="https://auth.example.com",
    audience="slim",
    jwks_ttl=datetime.timedelta(hours=1),
    claim_cache_ttl=datetime.timedelta(minutes=1),
    policy=slim_bindings.OidcPolicyConfig.CEL(
        expression='"admin" in claims.groups'
    ),
)

base = slim_bindings.new_insecure_server_config("127.0.0.1:46357")
server_config = slim_bindings.ServerConfig(
    **{**vars(base), "auth": slim_bindings.ServerAuthenticationConfig.OIDC(config=oidc)}
)
await service.run_server_async(server_config)
```

`policy` accepts `OidcPolicyConfig.CEL`, `OidcPolicyConfig.REGO` (which must define `package slim.auth` with `default allow = false`), or `OidcPolicyConfig.REGO_FILE`.

### JSON configuration

`new_config_from_json` accepts a full gRPC client config covering TLS material, backoff, and every authentication mode (`basic`, `static_jwt`, `jwt`, `spire`, `oidc`):

```json
{
  "endpoint": "http://127.0.0.1:46357",
  "tls": { "insecure": true },
  "auth": {
    "type": "oidc",
    "issuer_url": "https://auth.example.com",
    "client_id": "my-client",
    "client_secret": "s3cr3t",
    "audience": "slim",
    "policy": { "cel": "\"admin\" in claims.groups" }
  }
}
```

The schema matches the [client configuration schema](https://github.com/agntcy/slim/blob/slim-v2.0.0/crates/config/src/schema/client-config.schema.json) in the slim repository.

## SLIMRPC

The Python SDK includes SLIMRPC support for Protobuf-based RPC over SLIM. Install the `protoc-gen-slimrpc-python` plugin and add it to your `buf.gen.yaml` alongside the standard Python protobuf plugin. See the [SLIMRPC Compiler](./slimrpc/compiler.md) and the [Serving](./tutorials/slimrpc/tutorial-serve.md) and [Client](./tutorials/slimrpc/tutorial-client.md) tutorials.

## Examples

The [slim-bindings/python](https://github.com/agntcy/slim-bindings/tree/main/python) directory includes complete working examples:

| Example | Description |
|---|---|
| `examples/point_to_point` | 1:1 messaging with request/reply |
| `examples/group` | Group sessions with moderator/participant roles |
| `examples/slimrpc/simple` | Protobuf RPC over SLIM |

**Point-to-point:**

```bash
cd python
task examples:p2p:alice    # Receiver
task examples:p2p:no-mls:bob   # Sender (in another terminal)
```

**SLIMRPC** (requires a running SLIM node and generated proto code):

```bash
task examples:rpc:server
task examples:rpc:client
```

## Platform Support

| Platform | Architecture | Status |
|---|---|---|
| Linux | x86_64 | Supported |
| Linux | aarch64 | Supported |
| macOS | x86_64 | Supported |
| macOS | aarch64 (Apple Silicon) | Supported |
| Windows | x86_64 | Supported |

Wheels bundle the correct native library for each platform automatically.

## Building from Source

To build the Python SDK from the slim-bindings repository:

```bash
git clone https://github.com/agntcy/slim-bindings
cd slim-bindings/python

# Build and install in development mode
task build
```

To create distributable wheels:

```bash
task python:bindings:packaging
```

See the [python README](https://github.com/agntcy/slim-bindings/blob/main/python/README.md) for the full list of development tasks.

## Next Steps

- [Connecting to SLIM](./tutorials/tutorial-connect.md) — Initialise the service and connect to a node
- [Creating an App](./tutorials/tutorial-app.md) — Register an application identity
- [Creating a Session](./tutorials/tutorial-session.md) — Open a point-to-point or group session
- [SLIMRPC](./slimrpc/index.md) — Protobuf RPC over SLIM
