# SLIM JavaScript/TypeScript Bindings

JavaScript/TypeScript bindings for the SLIM (Secure Lightweight Identity Messaging) data plane, enabling secure messaging in React Native mobile apps and web browsers.

## Features

- 🦀 **Rust-Powered**: High-performance Rust core with TypeScript bindings
- 📱 **React Native**: Native mobile support for iOS and Android via JSI
- 🌐 **Web/WASM**: Browser support via WebAssembly (note: requires adapter modifications)
- 🔒 **Type-Safe**: Full TypeScript definitions with IntelliSense support
- 🔐 **Multiple Auth**: SharedSecret, JWT, SPIRE authentication support
- ⚡ **Async/Await**: Promise-based async operations
- 📦 **UniFFI**: Generated bindings via Mozilla's UniFFI framework

## Installation

### For React Native Projects

```bash
npm install @agntcy/slim-bindings
# or
yarn add @agntcy/slim-bindings
```

### For Web Projects

**Note**: WASM support is currently limited because the underlying Rust adapter uses `tokio`, which doesn't support `wasm32-unknown-unknown` target without conditional compilation. The TypeScript bindings are generated, but the WASM module cannot be compiled yet.

See [WASM Support Status](#wasm-support-status) for details.

## Quick Start

### Basic Usage

```typescript
import { 
  initializeCryptoProvider,
  createAppWithSecret,
  Name,
  SessionType,
  type SessionConfig
} from '@agntcy/slim-bindings'

// Initialize crypto provider (required once at startup)
initializeCryptoProvider()

// Create an app with shared secret authentication
const appName = new Name(['org', 'myapp', 'v1'], undefined)
const sharedSecret = 'your-secret-must-be-at-least-32-bytes-long!'
const app = createAppWithSecret(appName, sharedSecret)

console.log(`App created with ID: ${app.id()}`)

// Create a point-to-point session
const config: SessionConfig = {
  sessionType: SessionType.PointToPoint,
  enableMls: false,
  maxRetries: undefined,
  interval: undefined,
  metadata: {}
}

const destination = new Name(['org', 'receiver', 'v1'], undefined)

try {
  const session = await app.createSessionAndWait(config, destination)
  
  // Publish a message
  const message = new TextEncoder().encode('Hello from TypeScript!')
  await session.publishAndWait(message, undefined, undefined)
  
  console.log('Message sent successfully!')
  
  // Clean up
  await app.deleteSessionAndWait(session)
} catch (error) {
  console.error('Operation failed:', error)
} finally {
  app.destroy()
}
```

### React Native Example

```typescript
import React, { useEffect, useState } from 'react'
import { View, Text, Button } from 'react-native'
import { 
  initializeCryptoProvider,
  createAppWithSecret,
  Name,
  SessionType
} from '@agntcy/slim-bindings'

export default function App() {
  const [status, setStatus] = useState('Initializing...')
  
  useEffect(() => {
    initializeCryptoProvider()
    setStatus('Ready')
  }, [])
  
  const sendMessage = async () => {
    try {
      setStatus('Sending...')
      
      const app = createAppWithSecret(
        new Name(['org', 'mobile-app', 'v1'], undefined),
        'shared-secret-at-least-32-bytes-long-value!'
      )
      
      const config = {
        sessionType: SessionType.PointToPoint,
        enableMls: false,
        maxRetries: undefined,
        interval: undefined,
        metadata: {}
      }
      
      const session = await app.createSessionAndWait(
        config,
        new Name(['org', 'server', 'v1'], undefined)
      )
      
      const message = new TextEncoder().encode('Hello from React Native!')
      await session.publishAndWait(message, undefined, undefined)
      
      setStatus('Message sent!')
      
      await app.deleteSessionAndWait(session)
      app.destroy()
    } catch (error: any) {
      setStatus(`Error: ${error.message}`)
    }
  }
  
  return (
    <View style={{ flex: 1, justifyContent: 'center', alignItems: 'center' }}>
      <Text>SLIM React Native Demo</Text>
      <Text>{status}</Text>
      <Button title="Send Message" onPress={sendMessage} />
    </View>
  )
}
```

## API Reference

### Core Functions

#### `initializeCryptoProvider()`

Initialize the crypto provider. Must be called once before any other operations.

```typescript
initializeCryptoProvider()
```

#### `getVersion()`

Get the SLIM bindings version.

```typescript
const version = getVersion()
console.log(`Version: ${version}`)
```

#### `createAppWithSecret(name, secret)`

Create a new SLIM app with SharedSecret authentication.

```typescript
const app = createAppWithSecret(
  new Name(['org', 'app', 'v1'], undefined),
  'your-secret-at-least-32-bytes'
)
```

### Classes

#### `Name`

Represents a SLIM name with components and optional ID.

```typescript
// Constructor
const name = new Name(
  ['org', 'app', 'v1'],  // components: string[]
  BigInt(123)             // id?: bigint
)

// Methods
name.components(): string[]
name.id(): bigint
name.asString(): string
```

#### `BindingsAdapter`

Main app interface for managing sessions and operations.

```typescript
// Properties
app.id(): number
app.name(): Name

// Session Management
await app.createSessionAndWait(config: SessionConfig, destination: Name): Promise<BindingsSessionContext>
await app.deleteSessionAndWait(session: BindingsSessionContext): Promise<void>

// Subscription
await app.subscribe(name: Name, metadata?: Map<string, string>): Promise<void>
await app.unsubscribe(name: Name, metadata?: Map<string, string>): Promise<void>

// Cleanup
app.destroy(): void
```

#### `BindingsSessionContext`

Session context for messaging operations.

```typescript
// Messaging
await session.publishAndWait(
  data: Uint8Array,
  payloadType?: string,
  metadata?: Map<string, string>
): Promise<void>

// Receive messages (callback-based)
session.receive(callback: (message: ReceivedMessage) => void): void

// Group operations
await session.inviteAndWait(participant: Name): Promise<void>
await session.removeAndWait(participant: Name): Promise<void>
```

### Types

#### `SessionConfig`

Configuration for creating sessions.

```typescript
type SessionConfig = {
  sessionType: SessionType
  enableMls: boolean
  maxRetries?: number
  interval?: number
  metadata: Record<string, string>
}
```

#### `SessionType`

Enum for session types.

```typescript
enum SessionType {
  PointToPoint,  // 1:1 messaging
  Group          // Multicast messaging
}
```

#### `ReceivedMessage`

Received message structure.

```typescript
type ReceivedMessage = {
  data: Uint8Array
  context: MessageContext
}
```

## Development

### Prerequisites

- Node.js 18+
- Rust 1.70+
- Cargo
- Task (task runner)

### Building from Source

```bash
# Clone the repository
git clone https://github.com/agntcy/slim.git
cd slim/data-plane/bindings/javascript

# Install dependencies
npm install

# Generate bindings from Rust
task generate

# Run tests
task test

# Build for specific platforms
task generate:react-native  # React Native JSI bindings
task generate:wasm          # WASM bindings (TypeScript only)
```

### Running Tests

```bash
# All tests
npm test

# Unit tests only
npm run test:unit

# Integration tests only
npm run test:integration

# With coverage
npm run test:coverage
```

### Project Structure

```
javascript/
├── generated/           # Generated bindings (gitignored)
│   ├── typescript/     # TypeScript type definitions
│   ├── cpp/            # C++ JSI bindings (React Native)
│   └── wasm/           # WASM bindings
├── examples/
│   ├── react-native/   # React Native examples
│   │   ├── simple/
│   │   ├── point-to-point/
│   │   └── group/
│   └── web/            # Web/WASM examples
│       ├── simple/
│       ├── point-to-point/
│       └── group/
├── tests/
│   ├── unit.test.ts
│   └── integration.test.ts
├── package.json
├── tsconfig.json
├── vitest.config.ts
├── Taskfile.yaml
└── README.md
```

## WASM Support Status

**Current Status**: TypeScript bindings and WASM module structure are generated, but the WASM module cannot be compiled.

**Why**: The Rust adapter depends on `tokio` for async runtime, which doesn't support the `wasm32-unknown-unknown` target without significant modifications:

1. Tokio's `net` feature must be disabled for WASM
2. Many async operations need conditional compilation
3. System calls (file I/O, networking) aren't available in WASM

**Workarounds**:

1. **Feature Flags**: Add WASM-specific feature flags to the adapter crate
2. **Conditional Compilation**: Use `#[cfg(target_arch = "wasm32")]` to exclude incompatible code
3. **Alternative Runtime**: Use `wasm-bindgen-futures` instead of tokio for WASM builds
4. **Subset API**: Create a WASM-specific API that only exposes browser-compatible operations

**For Now**: The bindings work great for React Native. WASM support is a future enhancement that requires adapter refactoring.

## Examples

### Simple App Creation

See [`examples/react-native/simple/`](examples/react-native/simple/) for a complete example.

### Point-to-Point Messaging

See [`examples/react-native/point-to-point/`](examples/react-native/point-to-point/) for Alice/Bob example.

### Group Messaging

See [`examples/react-native/group/`](examples/react-native/group/) for moderator/participant example.

## Comparison with Go Bindings

| Feature | Go Bindings | JavaScript Bindings |
|---------|------------|---------------------|
| Type System | Go types | TypeScript types |
| Async Model | Channels & Goroutines | Promises & async/await |
| Platforms | Linux, macOS, Windows | React Native (iOS/Android), Web* |
| Memory Management | Go GC | JavaScript GC |
| Performance | Native | Native (RN) / WASM (Web) |
| Binding Tool | uniffi-bindgen-go | uniffi-bindgen-react-native |

*Web/WASM support requires adapter modifications

## Troubleshooting

### "Cannot find module" errors

Make sure bindings are generated:

```bash
task generate
```

### React Native linking issues

For iOS:

```bash
cd ios && pod install
```

For Android, ensure CMakeLists.txt is properly configured.

### WASM compilation fails

This is expected - see [WASM Support Status](#wasm-support-status).

### Tests fail with network errors

Integration tests expect failures without a running SLIM server. This is normal.

## Publishing (maintainers)

Releases are published to npm on tag `slim-bindings-*`. CI builds the iOS static lib, runs `task generate` and `task vendor:ios`, then `npm publish`. Ensure `NPM_TOKEN` is set in the repo. Dry run: run `task generate`, `task vendor:ios`, then `npm pack` and inspect the tarball.

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](../../../CONTRIBUTING.md) for guidelines.

## License

Apache-2.0 - see [LICENSE.md](../../../LICENSE.md)

## Resources

- **UniFFI Documentation**: https://mozilla.github.io/uniffi-rs/
- **uniffi-bindgen-react-native**: https://jhugman.github.io/uniffi-bindgen-react-native/
- **SLIM Project**: https://github.com/agntcy/slim
- **Go Bindings**: [`../go/`](../go/) (reference implementation)

## Support

For issues and questions:

- GitHub Issues: https://github.com/agntcy/slim/issues
- Discussions: https://github.com/agntcy/slim/discussions
