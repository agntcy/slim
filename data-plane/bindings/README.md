# SLIM Language Bindings

This directory contains language bindings for SLIM (Secure Low-Latency Interactive Messaging), organized for multi-language support.

## Structure

```
bindings/
├── common/          # Shared Rust implementation and UDL
│   ├── src/
│   │   ├── lib.rs              # Core implementation
│   │   ├── error.rs            # Error types
│   │   ├── slim_bindings.udl   # UniFFI interface definition
│   │   └── bin/
│   │       └── uniffi-bindgen.rs
│   ├── Cargo.toml
│   ├── build.rs
│   └── uniffi.toml
├── go/              # Go-specific bindings
│   ├── examples/
│   ├── generated/   # Generated Go code (created by uniffi-bindgen-go)
│   ├── Makefile
│   └── README.md
├── python/          # Python bindings (future)
└── javascript/      # JavaScript bindings (future)
```

## Language Support

### ✅ Go (Ready)

Complete Go bindings using UniFFI. See `go/README.md` for details.

**Quick Start:**
```bash
# Install uniffi-bindgen-go
cargo install uniffi-bindgen-go --git https://github.com/NordSecurity/uniffi-bindgen-go --tag v0.4.0+v0.28.3

# Build and generate
cd go
make all

# Run example
make example
```

### 🔜 Python (Future)

Python bindings can be generated from the same UDL file:
```bash
cd common
uniffi-bindgen generate src/slim_bindings.udl --language python --out-dir ../python/generated
```

### 🔜 JavaScript/TypeScript (Future)

JavaScript bindings using `uniffi-bindgen-ts`:
```bash
cd common
uniffi-bindgen-ts src/slim_bindings.udl --out-dir ../javascript/generated
```

## Common Rust Library

The `common/` directory contains the shared Rust implementation that all language bindings use:

- **`src/slim_bindings.udl`**: The UniFFI interface definition - single source of truth for all languages
- **`src/lib.rs`**: Rust implementation of the interface
- **`src/error.rs`**: Error type definitions
- **Cargo.toml**: Rust package configuration

### Building the Common Library

```bash
cd common
cargo build --release
```

This creates:
- `target/release/libslim_go_bindings.dylib` (macOS)
- `target/release/libslim_go_bindings.so` (Linux)
- `target/release/slim_go_bindings.dll` (Windows)

## Why This Structure?

### Single Source of Truth

The `common/slim_bindings.udl` file defines the interface **once**, and all language bindings are generated from it:

```
slim_bindings.udl  (one definition)
       ↓
       ├─→ uniffi-bindgen-go → Go bindings
       ├─→ uniffi-bindgen → Python bindings  
       └─→ uniffi-bindgen-ts → JavaScript bindings
```

### Benefits

1. **Consistency**: All languages have the exact same API
2. **Maintainability**: Update one file to change all bindings
3. **Clarity**: Interface is self-documenting in UDL
4. **Extensibility**: Easy to add new languages

## Adding a New Language

To add bindings for a new language:

1. **Create language directory:**
   ```bash
   mkdir -p bindings/newlang
   ```

2. **Generate bindings from the UDL:**
   ```bash
   cd common
   uniffi-bindgen generate src/slim_bindings.udl \
     --language newlang \
     --out-dir ../newlang/generated
   ```

3. **Add language-specific files:**
   - Build scripts/Makefiles
   - Examples
   - Documentation
   - Language-specific configuration

4. **Link to the compiled library:**
   - Point to `common/target/release/libslim_go_bindings.*`

## API Overview

All language bindings expose these core types:

- **Service**: Main service manager
- **App**: Application instance
- **SessionContext**: Active messaging session
- **Name**: Hierarchical identity
- **MessageContext**: Message metadata
- **SessionConfig**: Session configuration
- **SessionType**: PointToPoint or Multicast

See the UDL file (`common/src/slim_bindings.udl`) for the complete interface definition.

## Documentation

- **`common/src/slim_bindings.udl`**: Interface definition (API contract)
- **`go/README.md`**: Go bindings documentation
- **Language-specific READMEs**: In each language directory

## Development

### Modifying the API

1. Edit `common/src/slim_bindings.udl`
2. Update `common/src/lib.rs` implementation if needed
3. Rebuild: `cd common && cargo build --release`
4. Regenerate bindings for each language
5. Update language-specific examples/tests

### Testing Changes

```bash
# Build common library
cd common
cargo build --release
cargo test

# Test Go bindings
cd ../go
make clean
make all
make example

# Test other languages as they're added...
```

## Requirements

- **Rust**: 1.81+ (for building the common library)
- **UniFFI**: 0.28.3 (included as dependency)
- **Language-specific tools**: See individual language READMEs
