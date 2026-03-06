# SLIM JavaScript/TypeScript Bindings

JavaScript/TypeScript bindings for the SLIM data plane, enabling secure messaging in React Native mobile apps.

## Features

- **Rust-Powered**: High-performance Rust core with TypeScript bindings
- **React Native**: Native mobile support for iOS and Android via JSI
- **Type-Safe**: Full TypeScript definitions with IntelliSense support
- **Multiple Auth**: SharedSecret, JWT, SPIRE authentication support
- **Async/Await**: Promise-based async operations
- **UniFFI**: Generated bindings via Mozilla's UniFFI framework

## Installation

```bash
npm install @agntcy/slim-bindings
# or
yarn add @agntcy/slim-bindings
```

## Quick Start

1. Call `initializeCryptoProvider()` once at startup
2. Create an app with `createAppWithSecret(name, secret)` or other auth methods
3. Create sessions with `createSessionAndWait(config, destination)`
4. Publish messages with `session.publishAndWait(data, payloadType?, metadata?)`
5. Call `app.destroy()` when done

## API Overview

- **Core**: `initializeCryptoProvider()`, `getVersion()`, `createAppWithSecret(name, secret)`
- **Name**: `new Name(components, id?)` — components, id, asString
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

**"Cannot find module"** — Run `task generate`

**iOS undefined symbol errors** — XCFramework missing or pods need refresh:
```bash
task prepare:ios
cd examples/react-native/test-app/ios && pod install
```
Open `TestApp.xcworkspace` (not .xcodeproj), Clean Build Folder, then build.

**Android** — Ensure CMakeLists.txt is properly configured.

## Resources

- [UniFFI](https://mozilla.github.io/uniffi-rs/)
- [uniffi-bindgen-react-native](https://jhugman.github.io/uniffi-bindgen-react-native/)
- [SLIM Project](https://github.com/agntcy/slim)
- [Go Bindings](../go/)