# Supported Languages

The SLIM SDK is available for four languages. All bindings are generated from the same Rust core via [UniFFI](https://github.com/mozilla/uniffi-rs) and expose the same session layer and data plane client functionality with idiomatic APIs for each language.

## Python

| | |
|---|---|
| **Package** | `slim-bindings` on PyPI |
| **Install** | `pip install slim-bindings` |
| **Repository** | [github.com/agntcy/slim-bindings](https://github.com/agntcy/slim-bindings) |
| **Examples** | [python/examples](https://github.com/agntcy/slim-bindings/tree/main/python/examples) |
| **Requirements** | Python 3.9+ |

The Python bindings use async/await throughout. All session, publish, and receive operations have `_async` variants.

## Go

| | |
|---|---|
| **Package** | `slim-bindings-go` |
| **Install** | `go get github.com/agntcy/slim-bindings-go@v1.1.0` |
| **Repository** | [github.com/agntcy/slim-bindings-go](https://github.com/agntcy/slim-bindings-go) |
| **Examples** | [examples](https://github.com/agntcy/slim-bindings-go/tree/main/examples) |
| **Requirements** | Go 1.20+, C compiler (CGO) |

!!! warning "CGO Requirement"
    The Go bindings use native libraries via [CGO](https://pkg.go.dev/cmd/cgo). A C compiler (GCC or Clang) must be installed. Run the setup tool after installation:
    ```bash
    go run github.com/agntcy/slim-bindings-go/cmd/slim-bindings-setup
    ```

## Kotlin

| | |
|---|---|
| **Package** | `io.agntcy.slim:slim-bindings-kotlin` on Maven Central |
| **Install** | `implementation("io.agntcy.slim:slim-bindings-kotlin:1.2.0")` |
| **Repository** | [github.com/agntcy/slim-bindings](https://github.com/agntcy/slim-bindings) (kotlin subdirectory) |
| **Examples** | [kotlin/examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples) |
| **Requirements** | JDK 17+, JNA |

## .NET

| | |
|---|---|
| **Package** | `Agntcy.Slim.Bindings` on NuGet |
| **Install** | `dotnet add package Agntcy.Slim.Bindings` |
| **Repository** | [github.com/agntcy/slim-bindings](https://github.com/agntcy/slim-bindings) (dotnet subdirectory) |
| **Examples** | [dotnet/examples](https://github.com/agntcy/slim-bindings/tree/main/dotnet/examples) |

## Feature Parity

All bindings expose the same core feature set:

| Feature | Python | Go | Kotlin | .NET |
|---------|--------|-----|--------|------|
| Data plane client | ✓ | ✓ | ✓ | ✓ |
| Point-to-point sessions | ✓ | ✓ | ✓ | ✓ |
| Group sessions | ✓ | ✓ | ✓ | ✓ |
| MLS end-to-end encryption | ✓ | ✓ | ✓ | ✓ |
| Reliable delivery (ack/retry) | ✓ | ✓ | ✓ | ✓ |
| Async API | ✓ | ✓ | ✓ | ✓ |

## Installation Guide

See [SLIM SDK Installation](./install.md) for detailed per-language installation instructions.
