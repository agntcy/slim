# SLIM SDK — Language Bindings

All bindings are maintained in [agntcy/slim-bindings](https://github.com/agntcy/slim-bindings) and generated from the same Rust core via [UniFFI](https://github.com/mozilla/uniffi-rs). The Go binding is distributed through a separate [agntcy/slim-bindings-go](https://github.com/agntcy/slim-bindings-go) module because Go's module system requires source hosting rather than a package registry.

## Packages at a Glance

| Language | Package | Requirements | Install |
|---|---|---|---|
| [Python](./python.md) | [`slim-bindings`](https://pypi.org/project/slim-bindings/) on PyPI | Python 3.10+ | `pip install slim-bindings` |
| [Go](./go.md) | [`github.com/agntcy/slim-bindings-go/v3`](https://github.com/agntcy/slim-bindings-go) | Go 1.23+, C compiler (CGO) | `go get github.com/agntcy/slim-bindings-go/v3` |
| [.NET](./dotnet.md) | [`Agntcy.Slim`](https://www.nuget.org/packages/Agntcy.Slim) on NuGet | .NET 8.0+ | `dotnet add package Agntcy.Slim` |
| [Java](./java.md) | [`slim-bindings-java`](https://central.sonatype.com/artifact/io.agntcy.slim/slim-bindings-java) on Maven Central | JDK 21 with `--enable-preview`, Maven 3.8+ | Maven dependency |
| [Kotlin](./kotlin.md) | [`slim-bindings-kotlin`](https://central.sonatype.com/artifact/io.agntcy.slim/slim-bindings-kotlin) on Maven Central | JDK 17+, JNA | Gradle dependency |
| [Node.js](./node.md) | [`@agntcy/slim-bindings`](https://www.npmjs.com/package/@agntcy/slim-bindings) on npm | Node.js 18+ | `npm install @agntcy/slim-bindings` |
| [React Native](./react-native.md) | [`@agntcy/slim-bindings-react-native`](https://www.npmjs.com/package/@agntcy/slim-bindings-react-native) on npm | iOS or browser (WebAssembly) | `npm install @agntcy/slim-bindings-react-native` |

Each language guide has the full installation steps for that binding — exact version pins, build-tool snippets, and any post-install work — alongside its API overview, transport authentication, and platform support.

!!! warning "Post-install steps"
    Two bindings need one more step after the package is installed. Go requires `go run github.com/agntcy/slim-bindings-go/v3/cmd/slim-bindings-setup` (once per binding version) to fetch native libraries, and React Native requires `cd ios && pod install`. See the [Go](./go.md#installation) and [React Native](./react-native.md#installation) guides.

## Building from Source

To build the bindings from source:

```bash
git clone https://github.com/agntcy/slim-bindings
cd slim-bindings

# Build the Rust FFI library
cd rust && task build

# Build a specific binding (example: Python, installed in development mode)
cd ../python && task python:bindings:build
```

Each language guide has the build steps for its own binding, and the README in each binding directory covers the full list of development tasks.

## Next Steps

- [Connecting to SLIM](./tutorials/tutorial-connect.md) — Your first connection to a SLIM node
- [SLIM SDK Overview](./index.md) — Learn what the SDK provides
