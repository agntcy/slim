# SLIM JavaScript Bindings Examples

## React Native Example

Full React Native application demonstrating SLIM bindings usage via JSI.

**Location:** `examples/react-native/`

**Status:** ✅ Code example ready (requires full React Native project setup)

See [react-native/README.md](react-native/README.md) for details.

## Web Example

Web application demonstrating SLIM bindings usage via WASM.

**Location:** `examples/web/`

**Status:** ⚠️ Requires separate WASM bindings generation

See [web/README.md](web/README.md) for WASM generation instructions.

## Key Differences

### React Native (JSI)
- Native C++ bridge to Rust
- Full native performance
- iOS and Android support
- **Currently Generated** ✅

### Web (WASM)
- WebAssembly from Rust
- Runs in browser
- Cross-platform web support
- **Requires Additional Generation** ⚠️

## Requirements

### React Native
- React Native 0.73+
- Native build tools (Xcode/Android Studio)
- JSI module registration

### Web
- wasm-pack for building WASM
- Modern browser with WASM support
- HTTP server for local testing

## Features Demonstrated

Both examples show:
- Crypto provider initialization
- Version and build info retrieval
- SLIM app creation with shared secret authentication
- Error handling

## Current Status

✅ **React Native (JSI)** - Code examples ready  
⚠️ **Web (WASM)** - Requires WASM generation step (documented)
