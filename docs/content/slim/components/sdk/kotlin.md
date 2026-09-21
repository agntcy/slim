# Kotlin SDK

The SLIM Kotlin SDK (`io.agntcy.slim:slim-bindings-kotlin`) provides an idiomatic Kotlin/JVM API for building applications on SLIM. Bindings are generated from the same Rust core as every other language binding via [UniFFI](https://github.com/mozilla/uniffi-rs), with coroutine-based async/await and native libraries loaded through JNA.

## Requirements

| | |
|---|---|
| **Runtime** | JDK 17 or higher |
| **Package** | [`slim-bindings-kotlin`](https://central.sonatype.com/artifact/io.agntcy.slim/slim-bindings-kotlin) on Maven Central |
| **Build tools** | Gradle 8.5+ (wrapper included) |
| **Examples** | [kotlin/examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples) in slim-bindings |

The Maven artifact bundles native libraries for Linux, macOS, and Windows on x64 and arm64. JNA loads the correct library for your platform at runtime.

## Installation

=== "Gradle (Kotlin DSL)"

    Add to your `build.gradle.kts`:

    ```kotlin
    dependencies {
        implementation("io.agntcy.slim:slim-bindings-kotlin:1.0.0")
    }
    ```

=== "Gradle (Groovy)"

    ```groovy
    dependencies {
        implementation 'io.agntcy.slim:slim-bindings-kotlin:1.0.0'
    }
    ```

=== "Maven"

    ```xml
    <dependency>
      <groupId>io.agntcy.slim</groupId>
      <artifactId>slim-bindings-kotlin</artifactId>
      <version>1.0.0</version>
    </dependency>
    ```

## Quick Start

With a SLIM node running locally (see [Getting Started](../../slim-howto.md)), initialise the SDK, connect, and register an application identity:

```kotlin
import io.agntcy.slim.bindings.*
import kotlinx.coroutines.*

suspend fun main() = coroutineScope {
    initializeWithDefaults()
    val service = getGlobalService()

    val config = newInsecureClientConfig("http://127.0.0.1:46357")
    val connId = service.connectAsync(config)

    val localName = Name.fromString("myorg/default/my-service")
    val app = service.createAppWithSecret(
        localName, "change-me-before-going-to-production"
    )
    app.subscribeAsync(localName, connId)

    println("App ready: $localName, id=${app.id()}")
}
```

!!! note "Insecure mode"
    `newInsecureClientConfig` skips TLS and is for local development only. See [Authentication](../../architecture/authentication.md) for production TLS, mTLS, and SPIRE options.

Rust async functions are exposed as Kotlin `suspend` functions. Run them inside a coroutine scope (`runBlocking`, `coroutineScope`, or a framework like Ktor).

## API Overview

| Type | Description |
|---|---|
| Top-level functions | Static entry point for initialisation and global service access |
| `Service` | Manages connections and creates apps |
| `App` | Application handle for sessions, subscriptions, and routing |
| `Session` | Session for sending and receiving messages |
| `Name` | Identity in `org/namespace/app` format |
| `ReceivedMessage` | Received message with `payload` (bytes) and context metadata |
| `SessionConfig` | Session configuration (type, MLS, retries) |
| `ClientConfig` | Client connection configuration (endpoint, TLS, transport auth) |
| `ServerConfig` | Server listen configuration (endpoint, TLS, transport auth) |
| `OidcConfig` | OIDC transport authentication settings |
| `OidcPolicyConfig` | Claim-based access policy (`Cel`, `Rego`, `RegoFile`) |

Rust `Result<T, E>` types are converted to Kotlin exceptions. Catch `SlimException` subtypes for structured error handling.

### Initialisation and Connection

```kotlin
initializeWithDefaults()
val service = getGlobalService()

val config = newInsecureClientConfig("http://127.0.0.1:46357")
val connId = service.connectAsync(config)
```

### Sessions

Point-to-point sessions use `createSessionAsync`, which returns a completion handle:

```kotlin
val sessionConfig = SessionConfig(
    sessionType = SessionType.POINT_TO_POINT,
    enableMls = true,
    maxRetries = 5u,
    interval = Duration.ofSeconds(5),
    metadata = emptyMap(),
)

val sessionContext = app.createSessionAsync(sessionConfig, remoteName)
sessionContext.completion.waitAsync()
val session = sessionContext.session

session.publishAsync("Hello, SLIM!".toByteArray(), null, null)
```

## Transport Authentication

Separate from the app identity passed to `createAppWithSecret`, the gRPC connection to a SLIM node can carry its own credentials via `ClientConfig.auth`.

### OIDC (client credentials)

```kotlin
val clientConfig = newInsecureClientConfig("http://127.0.0.1:46357").apply {
    auth = ClientAuthenticationConfig.Oidc(
        OidcConfig(
            issuerUrl = "https://auth.example.com",
            clientId = "my-client",
            clientSecret = "s3cr3t",
            scope = "openid profile",
            timeout = Duration.ofSeconds(30),
        )
    )
}
val connId = service.connectAsync(clientConfig)
```

For the refresh-token flow, set `refreshToken` (or `refreshTokenFile`, which is rewritten in place as tokens rotate) instead of `clientSecret`.

### OIDC (server verification)

```kotlin
val serverConfig = newInsecureServerConfig("127.0.0.1:46357").apply {
    auth = ServerAuthenticationConfig.Oidc(
        OidcConfig(
            issuerUrl = "https://auth.example.com",
            audience = "slim",
            jwksTtl = Duration.ofHours(1),
            claimCacheTtl = Duration.ofMinutes(1),
            policy = OidcPolicyConfig.Cel(""""admin" in claims.groups"""),
        )
    )
}
service.runServerAsync(serverConfig)
```

`policy` accepts `OidcPolicyConfig.Cel`, `OidcPolicyConfig.Rego` (which must define `package slim.auth` with `default allow = false`), or `OidcPolicyConfig.RegoFile`.

### JSON configuration

`newConfigFromJson` accepts a full gRPC client config covering TLS material, backoff, and every authentication mode:

```json
{
  "endpoint": "http://127.0.0.1:46357",
  "tls": { "insecure": true },
  "auth": {
    "type": "oidc",
    "issuer_url": "https://auth.example.com",
    "client_id": "my-client",
    "client_secret": "s3cr3t",
    "audience": "slim",
    "policy": { "cel": "\"admin\" in claims.groups" }
  }
}
```

The schema matches the [client configuration schema](https://github.com/agntcy/slim/blob/slim-v2.0.0/crates/config/src/schema/client-config.schema.json) in the slim repository.

## SLIMRPC

The Kotlin SDK includes SLIMRPC support for Protobuf-based RPC over SLIM. Install the `protoc-gen-slimrpc-kotlin` plugin and add it to your `buf.gen.yaml` alongside the standard Java protobuf plugin. See the [SLIMRPC Compiler](./slimrpc/compiler.md) and the [Serving](./tutorials/slimrpc/tutorial-serve.md) and [Client](./tutorials/slimrpc/tutorial-client.md) tutorials.

Generated code lives in the `io.agntcy.slim.bindings.slimrpc` package.

## Examples

The [slim-bindings/kotlin](https://github.com/agntcy/slim-bindings/tree/main/kotlin) directory includes complete working examples:

| Example | Description |
|---|---|
| `PointToPoint.kt` | 1:1 messaging with request/reply |
| `Group.kt` | Group sessions with moderator/participant roles |
| `Server.kt` | SLIM data plane server |
| `examples/slimrpc/simple` | Protobuf RPC over SLIM |

**Point-to-point:**

```bash
cd kotlin
task examples:p2p:alice       # Receiver
task examples:p2p:no-mls:bob  # Sender (in another terminal)
```

**SLIMRPC** (requires a running SLIM node and generated proto code):

```bash
task examples:rpc:server
task examples:rpc:client
```

## Platform Support

| Platform | Architecture | Status |
|---|---|---|
| Linux | x86_64 | Supported |
| Linux | aarch64 | Supported |
| macOS | x86_64 | Supported |
| macOS | aarch64 (Apple Silicon) | Supported |
| Windows | x86_64 | Supported |

## Building from Source

To build the Kotlin SDK from the slim-bindings repository:

```bash
git clone https://github.com/agntcy/slim-bindings
cd slim-bindings/kotlin

task generate   # regenerate Kotlin bindings from Rust artifacts
task build
```

See the [kotlin README](https://github.com/agntcy/slim-bindings/blob/main/kotlin/README.md) for the full list of development tasks.

## Next Steps

- [Connecting to SLIM](./tutorials/tutorial-connect.md) — Initialise the service and connect to a node
- [Creating an App](./tutorials/tutorial-app.md) — Register an application identity
- [Creating a Session](./tutorials/tutorial-session.md) — Open a point-to-point or group session
- [SLIMRPC](./slimrpc/index.md) — Protobuf RPC over SLIM
