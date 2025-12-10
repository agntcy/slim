# Python Bindings Packaging Guide

This guide explains how to create distributable Python wheel packages for the SLIM UniFFI bindings.

## Overview

The `python:bindings:packaging` task creates self-contained wheel packages for multiple Python versions (3.10, 3.11, 3.12, 3.13) with the native Rust library bundled inside.

## Quick Start

To build all wheels with bundled native libraries:

```bash
cd data-plane/bindings/python
task python:bindings:packaging
```

## What Gets Created

The packaging task generates:

```
dist/
├── slim_uniffi_bindings-0.7.0-cp310-*.whl    # Python 3.10 wheel
├── slim_uniffi_bindings-0.7.0-cp311-*.whl    # Python 3.11 wheel
├── slim_uniffi_bindings-0.7.0-cp312-*.whl    # Python 3.12 wheel
├── slim_uniffi_bindings-0.7.0-cp313-*.whl    # Python 3.13 wheel
└── libslim_bindings.dylib                       # Standalone native library
```

## Packaging Process

The task performs these steps automatically:

### 1. Python Version Installation
```bash
uv python install 3.10
uv python install 3.11
uv python install 3.12
uv python install 3.13
```

### 2. Rust Library Build
```bash
cd ../adapter
cargo build --release
```

Produces: `../../target/release/libslim_bindings.dylib` (macOS) or `.so` (Linux)

### 3. Bindings Generation
```bash
uniffi-bindgen generate \
  --library ../../target/release/libslim_bindings.dylib \
  --language python \
  --out-dir slim_uniffi_bindings/generated
```

### 4. Native Library Bundling
```bash
cp ../../target/release/libslim_bindings.dylib \
   slim_uniffi_bindings/generated/
```

The native library is copied into the package so it's included in the wheel.

### 5. Wheel Building
```bash
uv build --python 3.10 --wheel
uv build --python 3.11 --wheel
uv build --python 3.12 --wheel
uv build --python 3.13 --wheel
```

### 6. Standalone Library Copy
```bash
cp ../../target/release/libslim_bindings.dylib dist/
```

## Advanced Usage

### Cross-Platform Builds

#### Build for Specific Architecture
```bash
# macOS ARM64 (Apple Silicon)
task python:bindings:packaging TARGET=aarch64-apple-darwin

# macOS x86_64 (Intel)
task python:bindings:packaging TARGET=x86_64-apple-darwin

# Linux x86_64
task python:bindings:packaging TARGET=x86_64-unknown-linux-gnu

# Linux ARM64
task python:bindings:packaging TARGET=aarch64-unknown-linux-gnu
```

#### Build in Debug Mode
```bash
task python:bindings:packaging PROFILE=debug
```

This is useful for development and debugging.

### Custom Builds

You can combine variables:

```bash
# Debug build for Apple Silicon
task python:bindings:packaging PROFILE=debug TARGET=aarch64-apple-darwin
```

## Distribution

### Publishing to PyPI

```bash
# Install twine
uv pip install twine

# Upload to PyPI
twine upload dist/*.whl

# Upload to Test PyPI
twine upload --repository testpypi dist/*.whl
```

### Local Installation

Users can install directly from the wheel:

```bash
pip install dist/slim_uniffi_bindings-0.7.0-*.whl
```

### Verification

After installation, verify it works:

```python
import slim_uniffi_bindings.generated.slim_bindings as slim

slim.initialize_crypto_provider()
print(f"SLIM Version: {slim.get_version()}")
```

## Technical Details

### Native Library Loading

The UniFFI-generated Python code automatically loads the native library from the same directory:

```python
# From slim_uniffi_bindings/generated/slim_bindings.py
def _uniffi_load_indirect():
    if sys.platform == "darwin":
        libname = "lib{}.dylib"
    elif sys.platform.startswith("win"):
        libname = "{}.dll"
    else:
        libname = "lib{}.so"
    
    libname = libname.format("slim_bindings")
    path = os.path.join(os.path.dirname(__file__), libname)
    lib = ctypes.cdll.LoadLibrary(path)
    return lib
```

### Package Structure

The wheel contains:

```
slim_uniffi_bindings/
├── __init__.py                              # Package entry point
├── py.typed                                 # Type hints marker
└── generated/
    ├── slim_bindings.py                     # UniFFI-generated bindings
    └── libslim_bindings.dylib               # Native library (bundled)
```

### pyproject.toml Configuration

The native library is included via:

```toml
[tool.hatch.build.targets.wheel]
packages = ["slim_uniffi_bindings"]
include = [
    "slim_uniffi_bindings/generated/*.dylib",
    "slim_uniffi_bindings/generated/*.so", 
    "slim_uniffi_bindings/generated/*.dll"
]
```

## Comparison with Maturin Approach

The maturin-based bindings (`data-plane/python/bindings/`) use a different approach:

| Aspect | UniFFI (this package) | Maturin |
|--------|----------------------|---------|
| Build Tool | `uniffi-bindgen` + `hatchling` | `maturin` |
| Bindings Type | UniFFI (FFI) | PyO3 (native Python) |
| Library Bundling | Manual (via `include` in `pyproject.toml`) | Automatic |
| Cross-language | ✅ (shared with Go, etc.) | ❌ (Python-only) |
| Build Complexity | Manual steps | Integrated |
| Wheel Type | `py3-none-any` (pure Python) | `cp310-cp310-*` (platform-specific) |

Both approaches are valid and have different trade-offs:

- **UniFFI**: Better for sharing code across multiple language bindings
- **Maturin**: Better for Python-specific optimizations and tighter integration

## Troubleshooting

### Library Not Found at Runtime

If you get `OSError: cannot load library` at runtime:

1. Verify the library is in the wheel:
   ```bash
   unzip -l dist/*.whl | grep libslim_bindings
   ```

2. Check the library extension matches your platform:
   - macOS: `.dylib`
   - Linux: `.so`
   - Windows: `.dll`

3. Ensure you built for the correct target architecture

### Build Failures

If the Rust build fails:

```bash
# Clean and rebuild
cd ../adapter
cargo clean
cargo build --release
```

### Missing Python Versions

If `uv python install` fails:

```bash
# Update uv
curl -LsSf https://astral.sh/uv/install.sh | sh

# List available Python versions
uv python list
```

## See Also

- [README.md](./README.md) - Main documentation
- [Taskfile.yaml](./Taskfile.yaml) - Build task definitions
- [pyproject.toml](./pyproject.toml) - Package configuration
- [Maturin bindings](../../python/bindings/) - Alternative PyO3-based approach

