# SLIM SDK — Language Bindings

All bindings are maintained in [agntcy/slim-bindings](https://github.com/agntcy/slim-bindings) and generated from the same Rust core via [UniFFI](https://github.com/mozilla/uniffi-rs). The Go binding is distributed through a separate [agntcy/slim-bindings-go](https://github.com/agntcy/slim-bindings-go) module because Go's module system requires source hosting rather than a package registry.

## Python

| | |
|---|---|
| **Package** | `slim-bindings` on PyPI |
| **Requirements** | Python 3.10+ |
| **Examples** | [python/examples](https://github.com/agntcy/slim-bindings/tree/main/python/examples) |

```bash
pip install slim-bindings
```

Or add to your `pyproject.toml`:

```toml
[project]
dependencies = ["slim-bindings"]
```

See the [Python SDK guide](./python.md) for API overview, transport authentication, SLIMRPC, examples, and platform support.

## Go

| | |
|---|---|
| **Package** | `github.com/agntcy/slim-bindings-go` |
| **Requirements** | Go 1.23+, C compiler (CGO) |
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

See the [Go SDK guide](./go.md) for API overview, transport authentication, SLIMRPC, examples, and platform support.

## .NET

| | |
|---|---|
| **Package** | [`Agntcy.Slim`](https://www.nuget.org/packages/Agntcy.Slim) on NuGet |
| **Requirements** | .NET 8.0+ |
| **Examples** | [dotnet/](https://github.com/agntcy/slim-bindings/tree/main/dotnet) |

```bash
dotnet add package Agntcy.Slim
```

Or add to your `.csproj`:

```xml
<PackageReference Include="Agntcy.Slim" Version="2.1.0" />
```

The NuGet package bundles native libraries for Linux, macOS, and Windows (x64 and arm64). No additional setup is required after install.

See the [.NET SDK guide](./dotnet.md) for API overview, transport authentication, SLIMRPC, examples, and platform support.

## Java

| | |
|---|---|
| **Package** | Maven Central |
| **Requirements** | Java 21+, Maven 3.8+, JNA |
| **Examples** | [java/examples](https://github.com/agntcy/slim-bindings/tree/main/java/examples) |

The Java bindings provide synchronous methods with `CompletableFuture`-based async variants.

Add to your `pom.xml`:

```xml
<dependency>
  <groupId>io.agntcy.slim</groupId>
  <artifactId>slim-bindings-java</artifactId>
  <version>1.2.0</version>
</dependency>
```

See the [Java SDK guide](./java.md) for API overview, transport authentication, SLIMRPC, examples, and platform support.

## Kotlin

| | |
|---|---|
| **Package** | Maven Central |
| **Requirements** | JDK 17+, JNA |
| **Examples** | [kotlin/examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples) |

Add to your `build.gradle.kts`:

```kotlin
dependencies {
    implementation("io.agntcy.slim:slim-bindings-kotlin:1.0.0")
}
```

See the [Kotlin SDK guide](./kotlin.md) for API overview, transport authentication, SLIMRPC, examples, and platform support.

## Node.js

| | |
|---|---|
| **Package** | `@agntcy/slim-bindings` on npm |
| **Requirements** | Node.js 18+ |

```bash
npm install @agntcy/slim-bindings
```

See the [Node.js SDK guide](./node.md) for API overview, transport authentication, SLIMRPC, examples, and platform support.

## React Native

| | |
|---|---|
| **Package** | `@agntcy/slim-bindings-react-native` on npm |
| **Requirements** | iOS or Android |

```bash
npm install @agntcy/slim-bindings-react-native
```

See the [React Native SDK guide](./react-native.md) for native and browser quick starts, platform support, and examples.

## Building from Source

To build the bindings from source:

```bash
git clone https://github.com/agntcy/slim-bindings
cd slim-bindings

# Build the Rust FFI library
cd rust && task build

# Build a specific binding (example: Python)
cd python && task build
```

See the README in each binding directory for language-specific build instructions.

## Next Steps

- [Connecting to SLIM](./tutorials/tutorial-connect.md) — Your first connection to a SLIM node
- [SLIM SDK Overview](./index.md) — Learn what the SDK provides
