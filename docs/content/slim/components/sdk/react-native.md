# React Native SDK

The SLIM React Native SDK (`@agntcy/slim-bindings-react-native`) provides mobile and browser APIs for building applications on SLIM. Bindings are generated from the same Rust core as every other language binding via [uniffi-bindgen-react-native](https://jhugman.github.io/uniffi-bindgen-react-native/), with native integration for iOS and Android and a WebAssembly build for browsers.

For Node.js servers, CLI tools, and non-mobile JavaScript applications, use [`@agntcy/slim-bindings`](./node.md) instead.

## Requirements

| | |
|---|---|
| **Native targets** | iOS, Android (React Native) |
| **Web target** | Modern browsers with WebAssembly support |
| **Package** | [`@agntcy/slim-bindings-react-native`](https://www.npmjs.com/package/@agntcy/slim-bindings-react-native) on npm |
| **Build tools** | Node.js 18+, Rust 1.70+ (for building from source) |
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

## Quick Start (React Native)

With a SLIM node running locally (see [Getting Started](../../slim-howto.md)), initialise the SDK, connect, and register an application identity:

```typescript
import slimBindings from '@agntcy/slim-bindings-react-native';

// Wait for JSI native module, then initialise (call once per app lifecycle)
await slimBindings.waitForJSIBindings(5000);
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

Call `app.destroy()` when finished to release native resources.

## Quick Start (Browser / WebAssembly)

Browser builds connect over `ws://` or `wss://` and use a separate entry point. Call `uniffiInitAsync()` once before using the bindings:

```typescript
import {
  App,
  Direction,
  Name,
  SessionType,
  uniffiInitAsync,
} from '@agntcy/slim-bindings-react-native/web';

await uniffiInitAsync();

const local = new Name('org', 'default', 'browser');
const remote = new Name('org', 'default', 'agent');

const app = await App.connectWithSecret(
  'wss://slim.example/ws',
  undefined,
  local,
  'change-me-before-going-to-production',
  Direction.Bidirectional,
);

await app.setRouteViaUpstreamAsync(remote);
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
  new TextEncoder().encode('Hello, SLIM!'),
  'text/plain',
  undefined,
);
```

All operations that wait for network or session state must use their async variants in browsers.

## API Overview

| Type | Description |
|---|---|
| Default export / `/web` entry | Static entry point for initialisation and global service access |
| `App` | Application handle for sessions, subscriptions, and routing |
| `BindingsSessionContext` | Session for sending and receiving messages |
| `Name` | Identity in `org/namespace/app` format |
| `ReceivedMessage` | Received message with payload and context metadata |
| `SessionConfig` | Session configuration (type, MLS, retries) |
| `SessionType` | `PointToPoint` or `Group` |

On the **native (iOS/Android)** target, the gRPC client and server configurations expose transport authentication via `config.auth`, matching the [Node.js SDK](./node.md#transport-authentication). On the **web/WebAssembly** target, gRPC configuration is absent by design — browser apps connect over WebSocket and authenticate through the identity path instead.

### Sessions (Native)

```typescript
const sessionConfig = {
  sessionType: slimBindings.SessionType.PointToPoint,
  enableMls: true,
  maxRetries: 5,
  interval: 5000,
  metadata: new Map(),
};

const sessionCtx = await app.createSessionAsync(sessionConfig, remoteName);
await sessionCtx.completion.waitAsync();
const session = sessionCtx.session;

await session.publishAsync(new TextEncoder().encode('Hello, SLIM!'), null, null);
```

## Transport Authentication

On the **native (iOS/Android)** target, transport authentication via OIDC and other gRPC auth modes matches the Node.js SDK. See the [Node.js transport authentication section](./node.md#transport-authentication) for code snippets and the JSON config form.

On the **web/WebAssembly** target, the browser build compiles out the gRPC configuration module (`ClientConfig`, `TlsClientConfig`, `ClientAuthenticationConfig`, and therefore OIDC). Browser apps authenticate through the shared-secret, JWT, or SPIRE identity path instead.

## Examples

The [slim-bindings/react-native](https://github.com/agntcy/slim-bindings/tree/main/react-native) directory includes:

| Example | Description |
|---|---|
| `examples/react-native/test-app` | React Native test app for iOS and Android |
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
| Native | Android | Supported |
| Web | Browsers with WASM | Supported |

## Building from Source

To build the React Native SDK from the slim-bindings repository:

```bash
git clone https://github.com/agntcy/slim-bindings
cd slim-bindings/react-native

npm install
task generate
task generate:web
task test
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
