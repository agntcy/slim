# SLIM React Native bindings (`@agntcy/slim-bindings-react-native`)

Use this package for **React Native** apps on **iOS and Android**. It exposes the SLIM data plane through UniFFI-generated bindings and the React Native native module toolchain (JSI, CocoaPods, Metro).

**Default JavaScript / TypeScript package** — For **Node.js** (servers, tooling, non-RN apps), install [`@agntcy/slim-bindings`](../node/README.md) instead. That is the primary npm entry point; this package is the mobile-focused variant.

## Features

- **iOS** — Native integration suited to React Native’s build and runtime model.
- **Shared SLIM surface** — Same general concepts as the Node bindings (sessions, messaging, auth helpers); implementation targets RN’s native layer rather than Node’s `ffi-rs` addon.
- **TypeScript** — Types ship with the package for application code.
- **UniFFI + RN tooling** — Generated with [uniffi-bindgen-react-native](https://jhugman.github.io/uniffi-bindgen-react-native/) workflows documented in this repo.

## Installation

```bash
npm install @agntcy/slim-bindings-react-native
```

```bash
yarn add @agntcy/slim-bindings-react-native
```

## Quick start

1. Call `initializeCryptoProvider()` once at startup.
2. Create an app with `createAppWithSecret(name, secret)` or another supported auth path.
3. Create sessions with `createSessionAndWait(config, destination)`.
4. Publish with `session.publishAndWait(data, payloadType?, metadata?)`.
5. Call `app.destroy()` when finished.

## API overview

- **Core**: `initializeCryptoProvider()`, `getVersion()`, `createAppWithSecret(name, secret)`
- **Name**: `new Name(components, id?)` — components, id, `asString`
- **BindingsAdapter**: `createSessionAndWait`, `deleteSessionAndWait`, `subscribe`, `unsubscribe`, `destroy`
- **BindingsSessionContext**: `publishAndWait`, `receive`, `inviteAndWait`, `removeAndWait`
- **Types**: `SessionConfig`, `SessionType` (PointToPoint, Group), `ReceivedMessage`

## Development

**Prerequisites**: Node.js 18+, Rust 1.70+, Task

```bash
git clone https://github.com/agntcy/slim.git
cd slim/data-plane/bindings/react-native
npm install
task generate
task test
```

## Troubleshooting

**“Cannot find module”** — Run `task generate`.

**iOS undefined symbol errors** — XCFramework or pods out of date:

```bash
task prepare:ios
cd examples/react-native/test-app/ios && pod install
```

Open `TestApp.xcworkspace` (not `.xcodeproj`), clean the build folder, then rebuild.

**Android** — Confirm CMake / NDK setup matches the project’s `CMakeLists.txt` expectations.

## Resources

- [Node / default JS bindings](../node/README.md) — `@agntcy/slim-bindings`
- [UniFFI](https://mozilla.github.io/uniffi-rs/)
- [uniffi-bindgen-react-native](https://jhugman.github.io/uniffi-bindgen-react-native/)
- [SLIM](https://github.com/agntcy/slim)
- [Go bindings](../go/)
