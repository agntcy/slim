# SLIM Common Bindings Library

This directory contains the **shared Rust implementation** and **UniFFI interface definition** used by all SLIM (Secure Low-Latency Interactive Messaging) language bindings.

## What's Here

- **`src/slim_bindings.udl`**: UniFFI interface definition (the API contract)
- **`src/lib.rs`**: Rust implementation
- **`src/error.rs`**: Error type definitions
- **`Cargo.toml`**: Rust package configuration
- **`build.rs`**: Build script
- **`uniffi.toml`**: UniFFI configuration

## Purpose

This library is **not meant to be used directly**. Instead:

1. Build this library to create the shared object (`.dylib`/`.so`/`.dll`)
2. Language-specific bindings (Go, Python, etc.) link to this library
3. The `slim_bindings.udl` file is used to generate language-specific code

## Building

### Release Build (Recommended)

```bash
cargo build --release
```

**Output:**
- macOS: `target/release/libslim_go_bindings.dylib`
- Linux: `target/release/libslim_go_bindings.so`
- Windows: `target/release/slim_go_bindings.dll`

### Debug Build

```bash
cargo build
```

**Output:**
- macOS: `target/debug/libslim_go_bindings.dylib`
- Linux: `target/debug/libslim_go_bindings.so`
- Windows: `target/debug/slim_go_bindings.dll`

## Testing

```bash
cargo test
```

## The UDL File

The `src/slim_bindings.udl` file defines the public API for all language bindings:

```udl
namespace slim_bindings {
    void initialize_crypto();
    string get_version();
};

interface Service { ... }
interface App { ... }
interface SessionContext { ... }

dictionary Name { ... }
dictionary MessageContext { ... }
dictionary SessionConfig { ... }

enum SessionType { ... }
enum SlimError { ... }
```

This file is the **single source of truth** - all language bindings are generated from it.

## Generating Language Bindings

### Go

```bash
uniffi-bindgen-go src/slim_bindings.udl --out-dir ../go/generated --config uniffi.toml
```

### Python

```bash
uniffi-bindgen generate src/slim_bindings.udl --language python --out-dir ../python/generated
```

### JavaScript/TypeScript

```bash
uniffi-bindgen-ts src/slim_bindings.udl --out-dir ../javascript/generated
```

## Architecture

```
┌─────────────────────────┐
│  Language Bindings      │
│  (Go, Python, JS, etc)  │
└────────────┬────────────┘
             │ FFI
             ▼
┌─────────────────────────┐
│  Common Rust Library    │  ← This directory
│  (slim_go_bindings)     │
└────────────┬────────────┘
             │
             ▼
┌─────────────────────────┐
│  SLIM Core              │
│  (slim_service, etc)    │
└─────────────────────────┘
```

## Modifying the API

When adding or changing the API:

1. **Update the UDL file** (`src/slim_bindings.udl`)
2. **Update the implementation** (`src/lib.rs`)
3. **Rebuild the library** (`cargo build --release`)
4. **Regenerate bindings** for each language
5. **Update documentation** and examples

### Example: Adding a New Method

**1. Add to UDL:**
```udl
interface App {
    // ... existing methods ...
    
    [Throws=SlimError, Async]
    void new_method(string param);
};
```

**2. Implement in Rust:**
```rust
impl App {
    // ... existing methods ...
    
    pub async fn new_method(&self, param: String) -> Result<()> {
        // Implementation
        Ok(())
    }
}
```

**3. Rebuild and regenerate bindings**

## Dependencies

The library depends on these SLIM core components:

- `agntcy-slim-auth`: Authentication
- `agntcy-slim-config`: Configuration
- `agntcy-slim-datapath`: Data plane
- `agntcy-slim-service`: Service layer
- `agntcy-slim-session`: Session management

These are specified as path dependencies in `Cargo.toml`.

## UniFFI Configuration

The `uniffi.toml` file configures UniFFI behavior:

```toml
[bindings.go]
package_name = "slimbindings"
module_name = "github.com/agntcy/slim/bindings"
```

This can be extended for other languages as needed.

## Compilation Features

Currently, the library is built as:
- `cdylib`: For dynamic linking (used by Go, Python, etc.)
- `staticlib`: For static linking if needed

## Notes

- This is a **workspace-independent** crate (has `[workspace]` in Cargo.toml)
- Uses UniFFI 0.28.3
- Requires Rust 1.81+
- All async operations use Tokio runtime

## Troubleshooting

### Build errors about missing dependencies

Make sure you're in the right directory and the core SLIM libraries are available:
```bash
cd /path/to/data-plane/bindings/common
cargo build --release
```

### UniFFI scaffolding errors

Clean and rebuild:
```bash
cargo clean
cargo build --release
```

### Path errors

The `Cargo.toml` uses relative paths to core SLIM libraries. Make sure the directory structure is:
```
data-plane/
├── bindings/
│   └── common/  ← You are here
└── core/
    ├── auth/
    ├── config/
    ├── datapath/
    └── ...
```
