# GoReleaser Cross-Compilation Setup

This document explains the cross-compilation setup for `slimctl` using goreleaser-cross pattern.

## Overview

The release workflow has been transformed to use a matrix-based approach inspired by [goreleaser/example-cross](https://github.com/goreleaser/example-cross), which enables proper CGO cross-compilation for multiple platforms.

## Architecture

### Three-Phase Release Process

1. **Prepare Phase** - Creates GitHub release and generates proto files
2. **Release Matrix Phase** - Builds for each platform in parallel
3. **Finalize Phase** - Merges artifacts and creates Homebrew cask

### Build Matrix

| Platform | Architecture | Runner | Build Method |
|----------|-------------|---------|--------------|
| macOS | amd64 | macos-15-intel | Native build |
| macOS | arm64 | macos-15 | Native build |
| Linux | amd64 | ubuntu-latest | goreleaser-cross (CGO) |
| Linux | arm64 | ubuntu-24.04-arm | goreleaser-cross (CGO) |
| Windows | amd64 | ubuntu-latest | goreleaser-cross (CGO) |

## How It Works

### Native Builds (macOS)
- Uses native runners for better performance
- CGO works natively on the platform
- Standard GoReleaser action

### Cross Builds (Linux, Windows)
- Uses [goreleaser-cross](https://github.com/goreleaser/goreleaser-cross) Docker container
- Container includes cross-compilation toolchains for CGO
- Properly handles C/C++ dependencies (Rust bindings via UniFFI)

### Split/Merge Strategy

GoReleaser v2 supports parallel builds using `--split` and `--merge`:

1. Each matrix job runs: `goreleaser release --split`
   - Builds only for its assigned platform
   - Uploads partial artifacts

2. Finalize job runs: `goreleaser continue --merge`
   - Downloads all partial artifacts
   - Merges them into final release
   - Generates checksums and Homebrew cask

## Key Benefits

✅ **Proper CGO handling** - goreleaser-cross provides complete toolchains
✅ **Parallel builds** - All platforms build simultaneously
✅ **Native performance** - macOS builds run on native hardware
✅ **Consistent environment** - Docker ensures reproducible Linux/Windows builds

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

## goreleaser-cross Container

The container (`goreleaser/goreleaser-cross:v1.23`) includes:
- Cross-compilation toolchains (gcc, g++, etc.)
- Zig compiler for additional cross-compilation
- Support for multiple architectures
- All necessary build tools

## References

- [goreleaser-cross repository](https://github.com/goreleaser/goreleaser-cross)
- [Example cross project](https://github.com/goreleaser/example-cross)
- [GoReleaser split/merge docs](https://goreleaser.com/customization/build/#split-and-merge)
