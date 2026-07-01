# SLIM SDK Installation

Install the SLIM language bindings for your target language. Each binding wraps the same Rust core and provides the full session layer and data plane client.

=== "Python"

    Install using pip:

    ```bash
    pip install slim-bindings
    ```

    Or add to your `pyproject.toml`:

    ```toml
    [project]
    dependencies = ["slim-bindings~=1.0"]
    ```

    **Requirements**: Python 3.9+

    For more information, see the [Python examples](https://github.com/agntcy/slim-bindings/tree/main/python/examples) in the slim-bindings repository.

=== "Go"

    Install the Go bindings:

    ```bash
    go get github.com/agntcy/slim-bindings-go@v1.1.0
    ```

    Run the setup tool to install native libraries:

    ```bash
    go run github.com/agntcy/slim-bindings-go/cmd/slim-bindings-setup
    ```

    Add to your `go.mod`:

    ```go
    require github.com/agntcy/slim-bindings-go v1.1.0
    ```

    !!! warning "C Compiler Required"
        The Go bindings use native libraries via [CGO](https://pkg.go.dev/cmd/cgo), so you'll need a C compiler installed on your system.

    For more information, see the [Go examples](https://github.com/agntcy/slim-bindings-go/tree/main/examples) in the repository.

=== "Kotlin"

    Add the Kotlin bindings to your Gradle project:

    === "Maven Central"

        Add to your `build.gradle.kts`:

        ```kotlin
        dependencies {
            implementation("io.agntcy.slim:slim-bindings-kotlin:1.2.0")
        }
        ```

        `mavenCentral()` is the default repository in Gradle, so no additional repository configuration is needed.

    === "GitHub Packages"

        Add the GitHub Packages repository and dependency to your `build.gradle.kts`:

        ```kotlin
        repositories {
            maven {
                url = uri("https://maven.pkg.github.com/agntcy/slim")
                credentials {
                    username = project.findProperty("gpr.user") as String? ?: System.getenv("GITHUB_ACTOR")
                    password = project.findProperty("gpr.key") as String? ?: System.getenv("GITHUB_TOKEN")
                }
            }
        }
        dependencies {
            implementation("io.agntcy.slim:slim-bindings-kotlin:1.2.0")
        }
        ```

        !!! note "GitHub Token Required"
            To use GitHub Packages, you need a personal access token with `read:packages` scope.

    !!! note "JDK 17+ Required"
        The Kotlin bindings use [JNA](https://github.com/java-native-access/jna) for native library loading and require JDK 17 or higher.

    For more information, see the [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples).

=== ".NET"

    Install from NuGet:

    ```bash
    dotnet add package Agntcy.Slim.Bindings
    ```

    For more information, see the [.NET examples](https://github.com/agntcy/slim-bindings/tree/main/dotnet/examples) in the slim-bindings repository.

## Building Bindings from Source

To build the bindings from source:

```bash
git clone https://github.com/agntcy/slim
cd slim

# Python (requires maturin and uv)
task python:bindings:build

# All language bindings
task data-plane:bindings:build
```

## Next Steps

- [Connecting to SLIM](./tutorial-connect.md) — Your first connection to a SLIM node
- [SLIM SDK Overview](./index.md) — Learn what the SDK provides
- [Supported Languages](./languages.md) — API documentation and feature parity details per language
