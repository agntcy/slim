# Node.js SDK

The SLIM Node.js SDK (`@agntcy/slim-bindings`) provides a TypeScript-friendly API for building applications on SLIM. Bindings are generated from the same Rust core as every other language binding via UniFFI, with native ESM modules and optional platform-specific native addons published to npm.

## Requirements

| | |
|---|---|
| **Runtime** | Node.js 18 or higher |
| **Package** | [`@agntcy/slim-bindings`](https://www.npmjs.com/package/@agntcy/slim-bindings) on npm |
| **Module format** | Native ESM (`import` only — `require()` is not supported) |
| **Examples** | [node/examples](https://github.com/agntcy/slim-bindings/tree/main/node/examples) in slim-bindings |

npm installs this package and, when published for your OS/arch, the matching optional native addon (`@agntcy/slim-bindings-*`). Full TypeScript types ship under `types/` in the published package.

## Installation

=== "npm"

    ```bash
    npm install @agntcy/slim-bindings
    ```

=== "yarn"

    ```bash
    yarn add @agntcy/slim-bindings
    ```

=== "pnpm"

    ```bash
    pnpm add @agntcy/slim-bindings
    ```

## Quick Start

With a SLIM node running locally (see [Getting Started](../../slim-howto.md)), initialise the SDK, connect, and register an application identity:

```typescript
import slimBindings from '@agntcy/slim-bindings';

slimBindings.initializeWithDefaults();
const service = slimBindings.getGlobalService();

const config = slimBindings.newInsecureClientConfig('http://127.0.0.1:46357');
const connId = await service.connectAsync(config);

const localName = new slimBindings.Name('myorg', 'default', 'my-service');
const app = service.createAppWithSecret(
  localName, 'change-me-before-going-to-production',
);
await app.subscribeAsync(localName, connId);

console.log(`App ready: ${localName}, id=${app.id()}`);
```

!!! note "Insecure mode"
    `newInsecureClientConfig` skips TLS and is for local development only. See [Authentication](../../architecture/authentication.md) for production TLS, mTLS, and SPIRE options.

## API Overview

| Type | Description |
|---|---|
| Default export | Static entry point for initialisation and global service access |
| `Service` | Manages connections and creates apps |
| `App` | Application handle for sessions, subscriptions, and routing |
| `Session` | Session for sending and receiving messages |
| `Name` | Identity in `org/namespace/app` format |
| `ReceivedMessage` | Received message with `payload` (bytes) and context metadata |
| `SessionConfig` | Session configuration (type, MLS, retries) |
| `ClientConfig` | Client connection configuration (endpoint, TLS, transport auth) |
| `ServerConfig` | Server listen configuration (endpoint, TLS, transport auth) |
| `OidcConfig` | OIDC transport authentication settings |
| `OidcPolicyConfig` | Claim-based access policy (`Cel`, `Rego`, `RegoFile`) |

### Type notes

64-bit values (like the connection ID from `connectAsync`) are real `bigint` end to end — pass them through as-is rather than converting to `Number`. Enum-typed fields (like `SessionConfig.sessionType`) are real TypeScript enums (`SessionType.PointToPoint`), not string literals.

### Initialisation and Connection

```typescript
slimBindings.initializeWithDefaults();
const service = slimBindings.getGlobalService();

const config = slimBindings.newInsecureClientConfig('http://127.0.0.1:46357');
const connId = await service.connectAsync(config);
```

### Sessions

Point-to-point sessions use `createSessionAsync`:

```typescript
const sessionConfig = new slimBindings.SessionConfig({
  sessionType: slimBindings.SessionType.PointToPoint,
  enableMls: true,
  maxRetries: 5,
  interval: 5000,  // milliseconds
  metadata: new Map(),
});

const sessionCtx = await app.createSessionAsync(sessionConfig, remoteName);
await sessionCtx.completion.waitAsync();
const session = sessionCtx.session;

await session.publishAsync(new TextEncoder().encode('Hello, SLIM!'), null, null);
```

## Transport Authentication

Separate from the app identity passed to `createAppWithSecret`, the gRPC connection to a SLIM node can carry its own credentials via `config.auth`.

### OIDC (client credentials)

```typescript
const config = slimBindings.newInsecureClientConfig('http://127.0.0.1:46357');
config.auth = new slimBindings.ClientAuthenticationConfig.Oidc({
  config: {
    issuerUrl: 'https://auth.example.com',
    clientId: 'my-client',
    clientSecret: 's3cr3t',
    scope: 'openid profile',
    timeout: 30_000,  // durations are milliseconds
  },
});

const connId = await service.connectAsync(config);
```

For the refresh-token flow, set `refreshToken` (or `refreshTokenFile`, which is rewritten in place as tokens rotate) instead of `clientSecret`.

### OIDC (server verification)

```typescript
const config = slimBindings.newInsecureServerConfig('0.0.0.0:46357');
config.auth = new slimBindings.ServerAuthenticationConfig.Oidc({
  config: {
    issuerUrl: 'https://auth.example.com',
    audience: 'slim',
    jwksTtl: 3_600_000,
    policy: new slimBindings.OidcPolicyConfig.Cel({
      expression: '"admin" in claims.groups',
    }),
  },
});

await service.runServerAsync(config);
```

`policy` accepts `OidcPolicyConfig.Cel`, `OidcPolicyConfig.Rego` (which must define `package slim.auth` with `default allow = false`), or `OidcPolicyConfig.RegoFile`.

### JSON configuration

`newConfigFromJson` accepts a full gRPC client config covering TLS material, backoff, and every authentication mode:

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

The Node.js SDK includes SLIMRPC support for Protobuf-based RPC over SLIM. Install the `protoc-gen-slimrpc-node` plugin and add it to your `buf.gen.yaml` alongside the standard protobuf-es plugin. See the [SLIMRPC Compiler](./slimrpc/compiler.md) and the [Serving](./tutorials/slimrpc/tutorial-serve.md) and [Client](./tutorials/slimrpc/tutorial-client.md) tutorials.

## Examples

The [slim-bindings/node](https://github.com/agntcy/slim-bindings/tree/main/node) directory includes complete working examples:

| Example | Description |
|---|---|
| `examples/point-to-point-alice.ts` | 1:1 messaging receiver |
| `examples/point-to-point-bob.ts` | 1:1 messaging sender |
| `examples/group.ts` | Group sessions with moderator/participant roles |
| `examples/slimrpc/simple` | Protobuf RPC over SLIM |

**Point-to-point:**

```bash
cd node
task example:alice   # Receiver
task example:bob     # Sender (in another terminal)
```

**SLIMRPC** (requires a running SLIM node and generated proto code):

```bash
task example:slimrpc:server
task example:slimrpc:client
```

## Platform Support

| Platform | Architecture | Status |
|---|---|---|
| Linux | x86_64 | Supported |
| Linux | aarch64 | Supported |
| macOS | x86_64 | Supported |
| macOS | aarch64 (Apple Silicon) | Supported |
| Windows | x86_64 | Supported |

If install fails with a native-load error, your platform/version combination may not have a published binary yet. Build from source or check the [node README](https://github.com/agntcy/slim-bindings/blob/main/node/README_dev.md) for maintainer instructions.

## Building from Source

To build the Node.js SDK from the slim-bindings repository:

```bash
git clone https://github.com/agntcy/slim-bindings
cd slim-bindings/node

npm install
task generate
task build
task test
```

See [README_dev.md](https://github.com/agntcy/slim-bindings/blob/main/node/README_dev.md) for generator setup, Task commands, and publishing.

## Next Steps

- [Connecting to SLIM](./tutorials/tutorial-connect.md) — Initialise the service and connect to a node
- [Creating an App](./tutorials/tutorial-app.md) — Register an application identity
- [Creating a Session](./tutorials/tutorial-session.md) — Open a point-to-point or group session
- [SLIMRPC](./slimrpc/index.md) — Protobuf RPC over SLIM
