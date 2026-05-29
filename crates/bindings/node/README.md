# `@agntcy/slim-bindings`

Node.js (≥18) bindings for **SLIM**: connect apps to a SLIM server, open sessions, and exchange messages using the same Rust core as other language bindings.

## Install

```bash
npm install @agntcy/slim-bindings
```

npm installs this package and, when published for your OS/arch, the matching optional native addon (`@agntcy/slim-bindings-*`). If install fails with a native-load error, your platform/version combo may not have a published binary yet.

## Module shape

The published entry loads the generated UniFFI/Node bindings (see `package.json` `main` / `types`). From TypeScript or ESM-aware tooling you typically import the default export from the package path your bundler resolves (examples in this repo import from `generated/slim-bindings-node.js` when running inside the bindings workspace).

```typescript
import slimBindings from '@agntcy/slim-bindings';

slimBindings.initializeWithDefaults();
const service = slimBindings.getGlobalService();
```

Full TypeScript types ship under `types/` in the published package (`index.d.ts` re-exports them).

## Typical flow

1. **Initialize** crypto/runtime once per process:

   `slimBindings.initializeWithDefaults()`

2. **Obtain the global service** — entry point for apps, connections, and server mode:

   `slimBindings.getGlobalService()`

3. **Identity** — SLIM names are three segments: `organization/namespace/application` (constructor `new slimBindings.Name(org, ns, app)`).

4. **Client: create an app** — e.g. shared-secret auth:

   `service.createAppWithSecret(name, secret)`  
   Use a secret that meets the minimum length required by your deployment (examples use a 32+ character demo string).

5. **Client: connect to the SLIM server** — build a client config (see `newInsecureClientConfig(url)` for development-style HTTP to the server), then:

   `await service.connectAsync(config)`  
   Returns a connection id used for routing/subscriptions.

6. **Subscribe** the app to receive traffic for its name (often passing `BigInt(connId)` if the generated API expects `bigint` — see note below).

7. **Server** — for a network node that accepts clients, initialize the same way, then build a server config (e.g. `newInsecureServerConfig('0.0.0.0:46357')`) and run:

   `slimBindings.getGlobalService().runServer(config)`  
   The process must stay alive while the server runs (see examples).

## Examples

Runnable scripts live under `examples/` (from repo root, use the `Taskfile` targets `example:server`, `example:alice`, `example:bob`, or run them via npm from `examples/` as documented in [README_dev.md](./README_dev.md)).

## `bigint` vs `number` (FFI)

Some APIs are typed with `bigint` for 64-bit values, but `connectAsync` may return a JavaScript `number` at runtime. When you pass that value into a method that expects `bigint` (for example `subscribeAsync`), wrap it: `BigInt(connId)`. See [README_dev.md](./README_dev.md#ffi-type-conversions) for details.

## Building from source / contributing

Generator setup, Task commands, publishing, and FFI patches are documented for maintainers in **[README_dev.md](./README_dev.md)**.

## Links

- [SLIM repository](https://github.com/agntcy/slim)
- [React Native bindings](../react-native/README.md) (mobile / JSI)
