# Java SDK

The SLIM Java SDK (`io.agntcy.slim:slim-bindings-java`) provides an idiomatic Java API for building applications on SLIM. Bindings are generated from the same Rust core as every other language binding via [uniffi-bindgen-java](https://github.com/IronCoreLabs/uniffi-bindgen-java), with synchronous methods and `CompletableFuture`-based async variants, and native libraries loaded through JNA.

## Requirements

| | |
|---|---|
| **Runtime** | Java 21 or higher |
| **Package** | [`slim-bindings-java`](https://central.sonatype.com/artifact/io.agntcy.slim/slim-bindings-java) on Maven Central |
| **Build tools** | Maven 3.8+ |
| **Examples** | [java/examples](https://github.com/agntcy/slim-bindings/tree/main/java/examples) in slim-bindings |

The Maven artifact bundles native libraries for Linux, macOS, and Windows on x64 and arm64. JNA loads the correct library for your platform at runtime.

## Installation

=== "Maven"

    Add to your `pom.xml`:

    ```xml
    <dependency>
      <groupId>io.agntcy.slim</groupId>
      <artifactId>slim-bindings-java</artifactId>
      <version>1.2.0</version>
    </dependency>
    ```

=== "Gradle"

    ```kotlin
    dependencies {
        implementation("io.agntcy.slim:slim-bindings-java:1.2.0")
    }
    ```

## Quick Start

With a SLIM node running locally (see [Getting Started](../../slim-howto.md)), initialise the SDK, connect, and register an application identity:

```java
import io.agntcy.slim.bindings.*;

public class Main {
    public static void main(String[] args) throws Exception {
        SlimBindings.initializeWithDefaults();
        Service service = SlimBindings.getGlobalService();

        ClientConfig config = SlimBindings.newInsecureClientConfig("http://127.0.0.1:46357");
        Long connId = service.connect(config);

        Name localName = Name.fromString("myorg/default/my-service");
        App app = service.createAppWithSecret(
            localName, "change-me-before-going-to-production"
        );
        app.subscribe(localName, connId);

        System.out.printf("App ready: %s, id=%s%n", localName, app.id());
    }
}
```

!!! note "Insecure mode"
    `newInsecureClientConfig` skips TLS and is for local development only. See [Authentication](../../architecture/authentication.md) for production TLS, mTLS, and SPIRE options.

## API Overview

| Class | Description |
|---|---|
| `SlimBindings` | Static entry point for initialisation and global service access |
| `Service` | Manages connections and creates apps |
| `App` | Application handle for sessions, subscriptions, and routing |
| `Session` | Session for sending and receiving messages |
| `Name` | Identity in `org/namespace/app` format |
| `ReceivedMessage` | Received message with `payload()` (bytes) and context metadata |
| `SessionConfig` | Session configuration (type, MLS, retries) |
| `ClientConfig` | Client connection configuration (endpoint, TLS, transport auth) |
| `ServerConfig` | Server listen configuration (endpoint, TLS, transport auth) |
| `OidcConfig` | OIDC transport authentication settings |
| `OidcPolicyConfig` | Claim-based access policy (`Cel`, `Rego`, `RegoFile`) |

Async variants (`*Async`) returning `CompletableFuture` are available for all operations. With Java 21 virtual threads, blocking on `.get()` or `.join()` is inexpensive.

### Initialisation and Connection

```java
SlimBindings.initializeWithDefaults();
Service service = SlimBindings.getGlobalService();

ClientConfig config = SlimBindings.newInsecureClientConfig("http://127.0.0.1:46357");
Long connId = service.connect(config);
```

For async connection:

```java
Long connId = service.connectAsync(config).get();
```

### Sessions

Point-to-point sessions use `createSessionAndWait`:

```java
SessionConfig sessionConfig = new SessionConfig(
    SessionType.POINT_TO_POINT,
    null,  // maxRetries
    null,  // interval
    Map.of(),
    new MlsSettings(100)
);

Session session = app.createSessionAndWait(sessionConfig, remoteName);
session.publishAndWait("Hello, SLIM!".getBytes(), null, null);
```

## Transport Authentication

Separate from the app identity passed to `createAppWithSecret`, the gRPC connection to a SLIM node can carry its own credentials via `ClientConfig.setAuth`.

### OIDC (client credentials)

```java
ClientConfig config = SlimBindings.newInsecureClientConfig("http://127.0.0.1:46357");
config.setAuth(new ClientAuthenticationConfig.Oidc(new OidcConfig(
    "https://auth.example.com",  // issuerUrl
    "my-client",                 // clientId
    "s3cr3t",                    // clientSecret
    null, null, null, null,      // audience, refreshToken, refreshTokenFile, accessTokenFile
    "openid profile",            // scope
    Duration.ofSeconds(30),        // timeout
    null, null, null)));          // jwksTtl, claimCacheTtl, policy

Long connId = service.connect(config);
```

For the refresh-token flow, set `refreshToken` (or `refreshTokenFile`, which is rewritten in place as tokens rotate) instead of `clientSecret`.

### OIDC (server verification)

```java
ServerConfig config = SlimBindings.newInsecureServerConfig("127.0.0.1:46357");
config.setAuth(new ServerAuthenticationConfig.Oidc(new OidcConfig(
    "https://auth.example.com",
    null, null,
    "slim",                      // audience — required for verification
    null, null, null, null, null,
    Duration.ofHours(1),         // jwksTtl
    Duration.ofMinutes(1),       // claimCacheTtl
    new OidcPolicyConfig.Cel("\"admin\" in claims.groups"))));

service.runServerAsync(config).get();
```

`policy` accepts `OidcPolicyConfig.Cel`, `OidcPolicyConfig.Rego` (which must define `package slim.auth` with `default allow = false`), or `OidcPolicyConfig.RegoFile`.

### JSON configuration

`SlimBindings.newConfigFromJson` accepts a full gRPC client config covering TLS material, backoff, and every authentication mode:

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

The Java SDK includes SLIMRPC support for Protobuf-based RPC over SLIM. Install the `protoc-gen-slimrpc-java` plugin and add it to your `buf.gen.yaml` alongside the standard Java protobuf plugin. See the [SLIMRPC Compiler](./slimrpc/compiler.md) and the [Serving](./tutorials/slimrpc/tutorial-serve.md) and [Client](./tutorials/slimrpc/tutorial-client.md) tutorials.

Generated code lives in the `io.agntcy.slim.bindings.slimrpc` package.

## Examples

The [slim-bindings/java](https://github.com/agntcy/slim-bindings/tree/main/java) directory includes complete working examples:

| Example | Description |
|---|---|
| `PointToPoint` | 1:1 messaging with request/reply |
| `Group` | Group sessions with moderator/participant roles |
| `Server` | SLIM data plane server |
| `examples/slimrpc/simple` | Protobuf RPC over SLIM |

**Point-to-point:**

```bash
cd java
task examples:p2p:alice   # Receiver
task examples:p2p:bob     # Sender (in another terminal)
```

**SLIMRPC** (requires a running SLIM node and generated proto code):

```bash
task examples:rpc:server
task examples:rpc:client
```

## Platform Support

| Platform | Architecture | JNA directory |
|---|---|---|
| Linux | x86_64 | `linux-x86-64` |
| Linux | aarch64 | `linux-aarch64` |
| macOS | x86_64 | `darwin-x86-64` |
| macOS | aarch64 | `darwin-aarch64` |
| Windows | x86_64 | `win32-x86-64` |
| Windows | aarch64 | `win32-aarch64` |

## Building from Source

To build the Java SDK from the slim-bindings repository:

```bash
git clone https://github.com/agntcy/slim-bindings
cd slim-bindings/java

task generate   # regenerate Java bindings from Rust artifacts
task build
task install    # install to local Maven repository
```

See the [java README](https://github.com/agntcy/slim-bindings/blob/main/java/README.md) for the full list of development tasks.

## Next Steps

- [Connecting to SLIM](./tutorials/tutorial-connect.md) — Initialise the service and connect to a node
- [Creating an App](./tutorials/tutorial-app.md) — Register an application identity
- [Creating a Session](./tutorials/tutorial-session.md) — Open a point-to-point or group session
- [SLIMRPC](./slimrpc/index.md) — Protobuf RPC over SLIM
