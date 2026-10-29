# React Native SDK

The SLIM React Native SDK (`@agntcy/slim-bindings-react-native`) provides mobile and browser APIs for building applications on SLIM. Bindings are generated from the same Rust core as every other language binding via [uniffi-bindgen-react-native](https://jhugman.github.io/uniffi-bindgen-react-native/), with native integration for iOS and a WebAssembly build for browsers. Android is not supported yet.

For Node.js servers, CLI tools, and non-mobile JavaScript applications, use [`@agntcy/slim-bindings`](./node.md) instead.

## Requirements

| | |
|---|---|
| **Native targets** | iOS (React Native); Android is not supported yet |
| **Web target** | Modern browsers with WebAssembly support |
| **Package** | [`@agntcy/slim-bindings-react-native`](https://www.npmjs.com/package/@agntcy/slim-bindings-react-native) on npm |
| **Build tools** | Node.js 18+, recent stable Rust, 1.88+ (for building from source) |
| **Examples** | [react-native/examples](https://github.com/agntcy/slim-bindings/tree/main/react-native/examples) in slim-bindings |

## Installation

=== "npm"

    ```bash
    npm install @agntcy/slim-bindings-react-native
    ```

=== "yarn"

    ```bash
    yarn add @agntcy/slim-bindings-react-native
    ```

After installing in a React Native project, run the iOS pod install step:

```bash
cd ios && pod install
```

## Getting Started

The [SDK tutorials](./tutorials/index.md) build a full application step by step — initialising the service, connecting to a node, creating an app, opening sessions, receiving messages, and adding persistence — with React Native snippets shown alongside every other binding. They target the native (iOS) build; the browser quick start below covers the WebAssembly entry point.

Start with [Connecting to SLIM](./tutorials/tutorial-connect.md).

## Browser / WebAssembly Quick Start

Browser builds connect over `ws://` or `wss://` and use a separate entry point. Call `uniffiInitAsync()` once before using the bindings:

```typescript
import {
  Direction,
  Name,
  Service,
  SessionType,
  uniffiInitAsync,
} from '@agntcy/slim-bindings-react-native/web';

await uniffiInitAsync();

const local = new Name('org', 'default', 'browser');
const remote = new Name('org', 'default', 'agent');

// The service id may contain only lowercase letters, digits and '-' (anything else panics).
const service = new Service('browser');

// Optional second argument: a token appended as the `token` query parameter.
const connId = await service.connectAsync('wss://slim.example/ws', undefined);

const app = await service.createAppWithDirectionAsync(
  local,
  'change-me-before-going-to-production',
  Direction.Bidirectional,
);

// Subscribe the app's own name so the node can route messages back to it,
// then route the remote name over the same connection.
await app.subscribeAsync(local, connId);
await app.setRouteAsync(remote, connId);

const session = await app.createSessionAndWaitAsync(
  {
    sessionType: SessionType.PointToPoint,
    maxRetries: 10,
    interval: undefined,
    metadata: new Map(),
    mlsSettings: undefined,
  },
  remote,
);

await session.publishAndWaitAsync(
  new TextEncoder().encode('Hello, SLIM!').buffer,
  'text/plain',
  undefined,
);
```

All operations that wait for network or session state must use their async variants in browsers. `service.createAppAsync(name, secret)` is a shorthand for a bidirectional app.

## API Overview

| Type | Description |
|---|---|
| Default export / `/web` entry | Static entry point for initialisation and global service access |
| `App` | Application handle for sessions, subscriptions, and routing |
| `Session` | Session for sending and receiving messages |
| `Name` | Identity in `org/namespace/app` format |
| `ReceivedMessage` | Received message with payload and context metadata |
| `SessionConfig` | Session configuration (type, MLS, retries) |
| `SessionType` | `PointToPoint` or `Group` |

On the **native (iOS)** target, the gRPC client and server configurations expose transport authentication via `config.auth`, matching the [Node.js SDK](./node.md#transport-authentication). On the **web/WebAssembly** target, gRPC configuration is absent by design — browser apps connect over WebSocket and authenticate through the identity path instead.

Call `app.uniffiDestroy()` when finished to release native resources.

### Session Configuration (Native)

```typescript
const sessionConfig: slimBindings.SessionConfig = {
  sessionType: slimBindings.SessionType.PointToPoint,
  metadata: new Map(),
  mlsSettings: { headerIntegrityValidationPercent: 100 },
};

const session = await app.createSessionAndWaitAsync(sessionConfig, remoteName);
```

Annotate the object as `SessionConfig` rather than typing it `any` — that is what catches an unknown field, which is otherwise accepted and silently ignored. `metadata` is required, `maxRetries` and `interval` fall back to the SLIM defaults, and omitting `mlsSettings` disables MLS.

## Transport Authentication

On the **native (iOS)** target, transport authentication via OIDC and other gRPC auth modes matches the Node.js SDK. See the [Node.js transport authentication section](./node.md#transport-authentication) for code snippets and the JSON config form.

On the **web/WebAssembly** target, the browser build compiles out the gRPC configuration module (`ClientConfig`, `TlsClientConfig`, `ClientAuthenticationConfig`, and therefore OIDC). Browser apps authenticate with a shared-secret app identity (`createAppAsync` / `createAppWithDirectionAsync`), optionally passing a token to `connectAsync`; JWT and SPIRE identities are native-only.

## Examples

The [slim-bindings/react-native](https://github.com/agntcy/slim-bindings/tree/main/react-native) directory includes:

| Example | Description |
|---|---|
| `examples/react-native/test-app` | React Native test app for iOS |
| `examples/browser` | Browser playground for unicast, multicast, and MLS |

**Browser playground:**

```bash
cd react-native
npm install
npm run build:web
cd examples/browser
npm install
npm run dev
```

See [examples/browser/README.md](https://github.com/agntcy/slim-bindings/blob/main/react-native/examples/browser/README.md) for point-to-point, group, and MLS walkthroughs.

## Platform Support

| Target | Platform | Status |
|---|---|---|
| Native | iOS | Supported |
| Native | Android | Not supported yet |
| Web | Browsers with WASM | Supported |

## Building from Source

To build the React Native SDK from the slim-bindings repository:

```bash
git clone https://github.com/agntcy/slim-bindings
cd slim-bindings/react-native

npm install
task generate       # React Native (JSI) bindings
task generate:web   # browser TypeScript bindings and WebAssembly package
task test:e2e:ios   # iOS E2E smoke tests (requires Xcode, an iOS simulator, and Maestro)
```

For iOS build issues, refresh the XCFramework and pods:

```bash
task prepare:ios
cd examples/react-native/test-app/ios && pod install
```

Open `TestApp.xcworkspace` (not `.xcodeproj`), clean the build folder, then rebuild.

See the [react-native README](https://github.com/agntcy/slim-bindings/blob/main/react-native/README.md) for troubleshooting and development tasks.

## Next Steps

- [Connecting to SLIM](./tutorials/tutorial-connect.md) — Initialise the service and connect to a node
- [Creating an App](./tutorials/tutorial-app.md) — Register an application identity
- [Creating a Session](./tutorials/tutorial-session.md) — Open a point-to-point or group session
- [Node.js SDK](./node.md) — Server-side and tooling use cases
