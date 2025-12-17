# Python Bindings Packaging Guide

This guide explains how to create distributable Python wheel packages for the SLIM UniFFI bindings using Maturin.

## Overview

The `python:bindings:packaging` task uses Maturin to create platform-specific wheel packages for multiple Python versions (3.10, 3.11, 3.12, 3.13) with the native Rust library automatically bundled inside.

## Quick Start

To build all wheels with bundled native libraries:

```bash
cd data-plane/bindings/python
task python:bindings:packaging
```

Or directly with Maturin:

```bash
uv run maturin build --release -i 3.10 3.11 3.12 3.13
```

## What Gets Created

The packaging task generates platform-specific wheels:

```
dist/
├── slim_uniffi_bindings-0.7.0-cp310-cp310-macosx_*.whl    # Python 3.10 wheel
├── slim_uniffi_bindings-0.7.0-cp311-cp311-macosx_*.whl    # Python 3.11 wheel
├── slim_uniffi_bindings-0.7.0-cp312-cp312-macosx_*.whl    # Python 3.12 wheel
└── slim_uniffi_bindings-0.7.0-cp313-cp313-macosx_*.whl    # Python 3.13 wheel
```

The wheel filename includes:
- Python version (cp310, cp311, etc.)
- Platform (macosx, manylinux, musllinux, win)
- Architecture (x86_64, arm64, etc.)

## Packaging Process

Maturin handles the entire build process automatically:

### 1. Python Version Installation
```bash
uv python install 3.10 3.11 3.12 3.13
```

### 2. Build with Maturin
```bash
uv run maturin build --release -i 3.10 3.11 3.12 3.13
```

Maturin automatically:
- Compiles the Rust UniFFI adapter library
- Generates Python bindings from UniFFI scaffolding
- Bundles the native library into the wheel
- Creates platform-specific wheels for each Python version

## Advanced Usage

### Cross-Platform Builds

#### Build for Specific Architecture
```bash
# macOS ARM64 (Apple Silicon)
task python:bindings:packaging TARGET=aarch64-apple-darwin

# macOS x86_64 (Intel)
task python:bindings:packaging TARGET=x86_64-apple-darwin

# Linux x86_64 with glibc
task python:bindings:packaging TARGET=x86_64-unknown-linux-gnu

# Linux x86_64 with musl
task python:bindings:packaging TARGET=x86_64-unknown-linux-musl

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

# Release build for Linux with specific Python versions
task python:bindings:packaging TARGET=x86_64-unknown-linux-gnu INTERPRETERS="3.11 3.12"
```

### Using Zig for Cross-Compilation

For Linux builds, Maturin can use Zig as a cross-compiler:

```bash
# Automatically enabled for gnu and musl targets
task python:bindings:packaging TARGET=x86_64-unknown-linux-gnu
```

This enables building Linux wheels from macOS or Windows.

## Distribution

### Publishing to PyPI

```bash
# Install twine
pip install twine

# Upload to PyPI
twine upload dist/*.whl

# Upload to Test PyPI
twine upload --repository testpypi dist/*.whl
```

### Local Installation

Users can install directly from the wheel:

```bash
pip install dist/slim_uniffi_bindings-0.7.0-cp310-*.whl
```

### Verification

After installation, verify it works:

```python
import slim_uniffi_bindings as slim

slim.initialize_crypto_provider()
print(f"SLIM Version: {slim.get_version()}")
```

## Technical Details

### Native Library Loading

Maturin automatically handles native library loading. The compiled extension module is directly importable as `_slim_bindings` and loaded via Python's import system.

### Package Structure

The wheel contains:

```
slim_uniffi_bindings/
├── __init__.py                              # Package entry point
├── py.typed                                 # Type hints marker
├── _slim_bindings.pyi                       # Type stubs (auto-generated)
└── _slim_bindings.cpython-310-*.so          # Native extension (bundled)
```

### pyproject.toml Configuration

The Maturin configuration is minimal:

```toml
[build-system]
requires = ["maturin>=1,<2"]
build-backend = "maturin"

[tool.maturin]
bindings = "uniffi"
manifest-path = "../adapter/Cargo.toml"
module-name = "slim_uniffi_bindings._slim_bindings"
```

## Maturin vs Manual UniFFI Approach

The new Maturin approach offers several advantages over the previous manual workflow:

| Aspect | Maturin (current) | Manual UniFFI (old) |
|--------|-------------------|---------------------|
| Build Tool | `maturin` (integrated) | `cargo` + `uniffi-bindgen` + `hatchling` |
| Bindings Generation | Automatic | Manual |
| Library Bundling | Automatic | Manual copy |
| Build Steps | 1 command | 4+ steps |
| Wheel Type | Platform-specific | Pure Python |
| Cross-compilation | Built-in (via Zig) | Manual setup |
| Consistency | Matches PyO3 bindings | Different workflow |

## Troubleshooting

### Library Not Found at Runtime

This should not happen with Maturin wheels, as the library is compiled into the extension module. If you get import errors:

1. Verify you installed the correct wheel for your platform
2. Check Python version matches the wheel (cp310, cp311, etc.)

### Build Failures

If the Rust build fails:

```bash
# Clean and rebuild
task clean
uv run maturin develop
```

### Missing Python Versions

If `uv python install` fails:

```bash
# Update uv
curl -LsSf https://astral.sh/uv/install.sh | sh

# List available Python versions
uv python list
```

### Cross-Compilation Issues

For cross-compilation to Linux targets:

1. Ensure Zig is installed (automatically via dependency groups)
2. Add the target: `rustup target add x86_64-unknown-linux-gnu`
3. Use the correct C library (gnu vs musl)

## Development Workflow

For development, use `maturin develop` instead of `maturin build`:

```bash
# Install in development mode (editable)
uv run maturin develop

# With release optimizations
uv run maturin develop --release
```

This is faster and allows iterative development.

## See Also

- [README.md](./README.md) - Main documentation
- [Taskfile.yaml](./Taskfile.yaml) - Build task definitions
- [pyproject.toml](./pyproject.toml) - Package configuration
- [Maturin Documentation](https://www.maturin.rs/) - Official Maturin docs
- [UniFFI Documentation](https://mozilla.github.io/uniffi-rs/) - UniFFI guide

