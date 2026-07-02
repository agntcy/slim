# SLIM SDK — Language Bindings

All bindings are maintained in [agntcy/slim-bindings](https://github.com/agntcy/slim-bindings) and generated from the same Rust core via [UniFFI](https://github.com/mozilla/uniffi-rs). The Go binding is distributed through a separate [agntcy/slim-bindings-go](https://github.com/agntcy/slim-bindings-go) module because Go's module system requires source hosting rather than a package registry.

## Python

| | |
|---|---|
| **Package** | `slim-bindings` on PyPI |
| **Requirements** | Python 3.9+ |
| **Examples** | [python/examples](https://github.com/agntcy/slim-bindings/tree/main/python/examples) |

```bash
pip install slim-bindings
```

Or add to your `pyproject.toml`:

```toml
[project]
dependencies = ["slim-bindings~=1.0"]
```

## Go

| | |
|---|---|
| **Package** | `github.com/agntcy/slim-bindings-go` |
| **Requirements** | Go 1.20+, C compiler (CGO) |
| **Examples** | [examples](https://github.com/agntcy/slim-bindings/tree/main/go/examples) |

```bash
go get github.com/agntcy/slim-bindings-go
```

Then run the setup tool to install the native libraries:

```bash
go run github.com/agntcy/slim-bindings-go/cmd/slim-bindings-setup
```

!!! warning "CGO Requirement"
    The Go bindings use native libraries via [CGO](https://pkg.go.dev/cmd/cgo). A C compiler (GCC or Clang) must be installed.

## .NET

| | |
|---|---|
| **Package** | `Agntcy.Slim` on NuGet |
| **Requirements** | .NET 8.0+ |
| **Examples** | [dotnet/](https://github.com/agntcy/slim-bindings/tree/main/dotnet) |

```bash
dotnet add package Agntcy.Slim
```

## Java

| | |
|---|---|
| **Package** | Maven Central |
| **Requirements** | Java 21+, Maven 3.8+, JNA |
| **Examples** | [java/examples](https://github.com/agntcy/slim-bindings/tree/main/java/examples) |

The Java bindings provide synchronous methods with `CompletableFuture`-based async variants.

## Kotlin

| | |
|---|---|
| **Package** | Maven Central |
| **Requirements** | JDK 17+, JNA |
| **Examples** | [kotlin/examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples) |

Add to your `build.gradle.kts`:

```kotlin
dependencies {
    implementation("io.agntcy.slim:slim-bindings-kotlin:1.2.0")
}
```

## Node.js

| | |
|---|---|
| **Package** | `@agntcy/slim-bindings` on npm |
| **Requirements** | Node.js 18+ |

```bash
npm install @agntcy/slim-bindings
```

## React Native

| | |
|---|---|
| **Package** | `@agntcy/slim-bindings-react-native` on npm |
| **Requirements** | iOS or Android |

```bash
npm install @agntcy/slim-bindings-react-native
```

## Building from Source

To build the bindings from source:

```bash
git clone https://github.com/agntcy/slim-bindings
cd slim-bindings

# Build the Rust FFI library
cd rust && task bindings:build

# Build a specific binding (example: Python)
cd python && task bindings:build
```

See the README in each binding directory for language-specific build instructions.

## Next Steps

- [Connecting to SLIM](./tutorial-connect.md) — Your first connection to a SLIM node
- [SLIM SDK Overview](./index.md) — Learn what the SDK provides
