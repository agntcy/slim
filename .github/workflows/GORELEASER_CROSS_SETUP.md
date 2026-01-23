# GoReleaser Cross-Compilation Setup

This document explains the cross-compilation setup for `slimctl` using goreleaser-cross.

## Overview

The release workflow has been transformed to use the [goreleaser-cross](https://github.com/goreleaser/goreleaser-cross) Docker container, which enables proper CGO cross-compilation for multiple platforms.

## Architecture

### Two-Phase Release Process

1. **Prepare Phase** - Creates GitHub release and generates proto files
2. **Build and Release Phase** - Uses goreleaser-cross to build all platforms with CGO support

### Supported Platforms

| Platform | Architecture | Build Method |
|----------|-------------|--------------|
| macOS | amd64 | goreleaser-cross (CGO) |
| macOS | arm64 | goreleaser-cross (CGO) |
| Linux | amd64 | goreleaser-cross (CGO) |
| Linux | arm64 | goreleaser-cross (CGO) |
| Windows | amd64 | goreleaser-cross (CGO) |

## How It Works

### goreleaser-cross Container

The `goreleaser/goreleaser-cross:v1.23` container provides:
- Complete cross-compilation toolchains for all platforms
- CGO support with proper C/C++ compilers
- Zig compiler for advanced cross-compilation
- Handles Rust FFI bindings (UniFFI) correctly

### Single-Job Build

Unlike the traditional approach, all platforms are built in a single job:

1. **Prepare job**: Creates GitHub release
2. **Build-and-release job**: 
   - Runs goreleaser inside the goreleaser-cross container
   - Container handles all cross-compilation automatically
   - Builds all platforms with proper CGO support
   - Uploads artifacts to the release
   - Generates Homebrew cask

## Key Benefits

✅ **Proper CGO handling** - goreleaser-cross provides complete toolchains
✅ **Consistent environment** - Docker ensures reproducible builds across all platforms
✅ **Simplified workflow** - Single job handles all platforms
✅ **Rust FFI support** - Works correctly with slim-bindings (UniFFI)

## Files Modified

- `.github/workflows/release-slimctl.yaml` - Matrix-based workflow
- `.goreleaser.yaml` - Updated for split/merge mode

## Usage

Release is triggered by pushing a tag:

```bash
git tag slimctl-v1.0.0
git push origin slimctl-v1.0.0
```

The workflow will:
1. Create the GitHub release
2. Build binaries for all platforms in parallel
3. Upload artifacts to the release
4. Create PR to update Homebrew cask

## Container Details

The `goreleaser/goreleaser-cross:v1.23` container includes:
- **Cross-compilation toolchains**: GCC, G++, Clang for all target platforms
- **Zig compiler**: Modern cross-compilation support
- **Multi-architecture support**: x86_64, ARM64, ARM, 386
- **CGO toolchains**: Complete C/C++ build environments
- **Rust support**: Can handle FFI bindings through UniFFI

## Workflow Steps

1. Tag is pushed: `slimctl-v1.0.0`
2. Prepare job creates GitHub release
3. Build-and-release job:
   - Checks out code with full history
   - Generates protobuf code
   - Runs goreleaser-cross container
   - Container builds all platforms with CGO
   - Uploads all artifacts to GitHub release
   - Creates Homebrew cask PR

## References

- [goreleaser-cross repository](https://github.com/goreleaser/goreleaser-cross)
- [Example cross project](https://github.com/goreleaser/example-cross)
- [GoReleaser documentation](https://goreleaser.com/)
