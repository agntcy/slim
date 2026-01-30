# slimrpc-compiler Multi-Language Support Design

## Overview

Refactor the slimrpc-compiler to support multiple target languages, starting with Python and Go. The current implementation only supports Python code generation.

## Current State

```
slimrpc-compiler/
├── Cargo.toml
├── src/
│   └── bin/
│       └── main.rs          # Single binary: protoc-slimrpc-plugin
```

**Current binary:**
- Name: `protoc-slimrpc-plugin`
- Target: Python only
- All templates and logic in `main.rs` (~991 lines)

## Proposed Architecture

```
slimrpc-compiler/
├── Cargo.toml
├── src/
│   ├── lib.rs               # Shared library code
│   ├── common/
│   │   └── mod.rs           # Common utilities (type resolution, I/O, etc.)
│   ├── python/
│   │   ├── mod.rs           # Python code generator
│   │   ├── templates.rs     # Python templates (stubs, servicers, handlers)
│   │   └── generator.rs     # Python generation logic
│   ├── golang/
│   │   ├── mod.rs           # Go code generator
│   │   ├── templates.rs     # Go templates (interfaces, implementations)
│   │   └── generator.rs     # Go generation logic
│   └── bin/
│       ├── protoc-gen-slimrpc-python.rs
│       └── protoc-gen-slimrpc-go.rs
```

## Module Breakdown

### 1. `src/lib.rs`
- Re-export common functionality
- Re-export language-specific modules

### 2. `src/common/mod.rs`
Shared utilities across all language generators:
- `read_request()` - Read CodeGeneratorRequest from stdin
- `write_response()` - Write CodeGeneratorResponse to stdout
- Common protobuf parsing logic
- File descriptor traversal
- Parameter parsing

### 3. `src/python/mod.rs`
Python-specific code generation:
- All current Python templates (stubs, servicers, handlers)
- `generate()` function that takes CodeGeneratorRequest and returns CodeGeneratorResponse
- Python-specific type resolution
- Import management

**Key components:**
- Client stub generation (async methods)
- Server servicer interface
- Server handler classes (serialization/deserialization)
- Registration functions

### 4. `src/golang/mod.rs`
Go-specific code generation:
- Go templates (interfaces, implementations)
- `generate()` function matching Python API
- Go-specific type resolution (proto to Go types)
- Package/import management

**Key components to implement:**
- Client stub generation (Go interfaces)
- Server handler interfaces
- Registration functions
- Proper Go naming conventions (PascalCase)

### 5. Binaries

#### `src/bin/protoc-gen-slimrpc-python.rs`
```rust
use agntcy_protoc_slimrpc_plugin::common;
use agntcy_protoc_slimrpc_plugin::python;

fn main() -> anyhow::Result<()> {
    let request = common::read_request()?;
    let response = python::generate(request)?;
    common::write_response(&response)?;
    Ok(())
}
```

#### `src/bin/protoc-gen-slimrpc-go.rs`
```rust
use agntcy_protoc_slimrpc_plugin::common;
use agntcy_protoc_slimrpc_plugin::golang;

fn main() -> anyhow::Result<()> {
    let request = common::read_request()?;
    let response = golang::generate(request)?;
    common::write_response(&response)?;
    Ok(())
}
```

## Cargo.toml Changes

```toml
[package]
name = "agntcy-protoc-slimrpc-plugin"
description = "A protoc plugin for generating slimrpc code"
version = "0.2.0"  # Bump for multi-language support
license.workspace = true
edition.workspace = true

[lib]
name = "agntcy_protoc_slimrpc_plugin"
path = "src/lib.rs"

[[bin]]
name = "protoc-gen-slimrpc-python"
path = "src/bin/protoc-gen-slimrpc-python.rs"

[[bin]]
name = "protoc-gen-slimrpc-go"
path = "src/bin/protoc-gen-slimrpc-go.rs"

[dependencies]
anyhow = { workspace = true }
heck = "0.5.0"
prost = "0.14"
prost-types = "0.14"
```

## Migration Strategy

### Phase 1: Extract Common Code
1. Create `src/lib.rs` with basic structure
2. Create `src/common/mod.rs` with I/O functions
3. Move shared utilities from main.rs

### Phase 2: Create Python Module
1. Create `src/python/mod.rs`
2. Move all Python templates to Python module
3. Extract Python generation logic
4. Create `generate()` function

### Phase 3: Create Python Binary
1. Create `src/bin/protoc-gen-slimrpc-python.rs`
2. Test that it works with existing examples
3. Update buf.gen.yaml to use new binary name

### Phase 4: Create Go Module
1. Create `src/golang/mod.rs`
2. Design Go templates based on slim_bindings Go API
3. Implement Go generation logic
4. Create `generate()` function

### Phase 5: Create Go Binary
1. Create `src/bin/protoc-gen-slimrpc-go.rs`
2. Test with Go examples
3. Create buf.gen.yaml for Go

### Phase 6: Testing & Documentation
1. Update all tests to use new structure
2. Test both Python and Go generation
3. Update README.md
4. Update examples

## API Design

### Common Module
```rust
pub fn read_request() -> Result<CodeGeneratorRequest>
pub fn write_response(response: &CodeGeneratorResponse) -> Result<()>
pub fn parse_parameters(param_str: &str) -> Result<HashMap<String, String>>
```

### Language Module Interface
Each language module should expose:
```rust
pub fn generate(request: CodeGeneratorRequest) -> Result<CodeGeneratorResponse>
```

## Go Code Generation Design

### Go Package Structure
Generated Go code should follow:
```
types/
├── example_pb2.pb.go        # Standard protobuf (from protoc-gen-go)
└── example_pb2_slimrpc.go   # slimrpc generated code
```

### Go Templates (Initial Design)

#### Client Stub
```go
type TestClient struct {
    channel *slim_bindings.Channel
}

func NewTestClient(channel *slim_bindings.Channel) *TestClient {
    return &TestClient{channel: channel}
}

func (c *TestClient) ExampleUnaryUnary(ctx context.Context, req *ExampleRequest) (*ExampleResponse, error) {
    // Implementation
}
```

#### Server Interface
```go
type TestServer interface {
    ExampleUnaryUnary(ctx context.Context, req *ExampleRequest) (*ExampleResponse, error)
    ExampleUnaryStream(req *ExampleRequest, stream TestExampleUnaryStreamServer) error
    // ...
}
```

## Naming Convention

- **Python binary:** `protoc-gen-slimrpc-python`
- **Go binary:** `protoc-gen-slimrpc-go`
- **Future binaries:** `protoc-gen-slimrpc-{language}`

## Backward Compatibility

**Breaking Change:** The binary name changes from `protoc-slimrpc-plugin` to `protoc-gen-slimrpc-python`

**Migration path:**
1. Update all buf.gen.yaml files to use new binary name
2. Provide deprecation notice in old binary (could symlink for transition)
3. Update documentation

## Testing Strategy

1. **Unit tests:** Test each module independently
2. **Integration tests:** Test full protoc workflow
3. **Golden file tests:** Compare generated output with expected output
4. **Cross-language tests:** Ensure Python client can talk to Go server

## Documentation Updates

1. Update main README.md with multi-language support
2. Create language-specific generation docs
3. Update examples to show both Python and Go
4. Document migration from old binary name

## Success Criteria

- [ ] Python binary generates identical code to current version
- [ ] Go binary generates working Go code
- [ ] All existing tests pass
- [ ] New tests for Go generation pass
- [ ] Documentation updated
- [ ] Examples updated

## Future Extensions

- TypeScript/JavaScript support
- Rust support (native slim bindings)
- Java support
- Per-language feature flags
- Custom template support
