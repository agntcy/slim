# Tutorial: Connecting to SLIM

This tutorial walks through the two steps required before your application can send or receive messages: initialising the SLIM service and connecting to a SLIM node.

## Prerequisites

- A running SLIM node (see [SLIM Data Plane Installation](../data-plane/install.md))
- The SLIM SDK installed for your language (see [SLIM SDK Installation](./install.md))

## Step 1: Initialise the Service

The SLIM service is the global runtime that manages connections and application state. It must be initialised once per process before any other SLIM calls.

=== "Python"

    The simplest initialisation uses `initialize_with_defaults()`, which sets up the runtime with sensible defaults for tracing and threading:

    ```python
    import asyncio
    import slim_bindings

    async def main():
        # Required for UniFFI async bindings
        slim_bindings.uniffi_set_event_loop(asyncio.get_running_loop())

        # Initialise the global SLIM service with defaults
        slim_bindings.initialize_with_defaults()

        # Obtain a reference to the global service
        service = slim_bindings.get_global_service()

    asyncio.run(main())
    ```

    For more control over tracing, threading, and service options use `initialize_with_configs`:

    ```python
    tracing_config = slim_bindings.new_tracing_config()
    tracing_config.log_level = "info"

    runtime_config = slim_bindings.new_runtime_config()
    service_config = slim_bindings.new_service_config()

    slim_bindings.initialize_with_configs(
        tracing_config=tracing_config,
        runtime_config=runtime_config,
        service_config=[service_config],
    )

    service = slim_bindings.get_global_service()
    ```

=== "Go"

    ```go
    import slim "github.com/agntcy/slim-bindings-go"

    func main() {
        // Initialise the global SLIM service with defaults
        slim.InitializeWithDefaults()
    }
    ```

=== "Java"

    ```java
    import io.agntcy.slim.bindings.*;

    public class Main {
        public static void main(String[] args) {
            // Initialise the global SLIM service with defaults
            SlimBindings.initializeWithDefaults();

            // Obtain a reference to the global service
            Service service = SlimBindings.getGlobalService();
        }
    }
    ```

=== "Kotlin"

    ```kotlin
    import io.agntcy.slim.bindings.*

    fun main() {
        // Initialise the global SLIM service with defaults
        initializeWithDefaults()

        // Obtain a reference to the global service
        val service = getGlobalService()
    }
    ```

=== "Node.js"

    ```typescript
    import slimBindings from '@agntcy/slim-bindings';

    // Initialise the global SLIM service with defaults
    slimBindings.initializeWithDefaults();

    // Obtain a reference to the global service
    const service = slimBindings.getGlobalService();
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim;

    // Initialise the global SLIM service with defaults
    Slim.Initialize();

    // Obtain a reference to the global service
    using var service = Slim.GetGlobalService();
    ```

=== "React Native"

    ```tsx
    import slimBindings from '@agntcy/slim-bindings-react-native';

    // Wait for JSI native module, then initialise (call once per app lifecycle)
    await slimBindings.waitForJSIBindings(5000);
    slimBindings.initializeWithDefaults();

    // Obtain a reference to the global service
    const service = slimBindings.getGlobalService();
    ```

## Step 2: Connect to a SLIM Node

With the service initialised, connect to a SLIM node. The connection returns a `conn_id` that is used in subsequent calls to identify which node an app or subscription is associated with.

=== "Python"

    ```python
    # Build a client config pointing at your SLIM node
    client_config = slim_bindings.new_insecure_client_config("http://127.0.0.1:46357")

    # Connect — returns a connection ID used in later calls
    conn_id = await service.connect_async(client_config)

    print(f"Connected, conn_id={conn_id}")
    ```

    For TLS-secured connections, build the client config with certificate paths instead:

    ```python
    client_config = slim_bindings.new_client_config(
        endpoint="https://slim.example.com:46357",
        ca_file="/path/to/ca.pem",
    )
    conn_id = await service.connect_async(client_config)
    ```

=== "Go"

    ```go
    // Build a client config pointing at your SLIM node
    config := slim.NewInsecureClientConfig("http://127.0.0.1:46357")

    // Connect — returns a connection ID used in later calls
    connID, err := slim.GetGlobalService().ConnectAsync(config)
    if err != nil {
        log.Fatal(err)
    }

    fmt.Printf("Connected, connID=%d\n", connID)
    ```

=== "Java"

    ```java
    // Build a client config pointing at your SLIM node
    ClientConfig config = SlimBindings.newInsecureClientConfig("http://127.0.0.1:46357");

    // Connect — returns a connection ID used in later calls
    Long connId = service.connect(config);

    System.out.println("Connected, connId=" + connId);
    ```

=== "Kotlin"

    ```kotlin
    // Build a client config pointing at your SLIM node
    val clientConfig = newInsecureClientConfig("http://127.0.0.1:46357")

    // Connect — returns a connection ID used in later calls
    val connId: ULong = service.connectAsync(clientConfig)

    println("Connected, connId=$connId")
    ```

=== "Node.js"

    ```typescript
    // Build a client config pointing at your SLIM node
    const config = slimBindings.newInsecureClientConfig("http://127.0.0.1:46357");

    // Connect — returns a connection ID used in later calls
    const connId = await service.connectAsync(config);

    console.log(`Connected, connId=${connId}`);
    ```

=== ".NET"

    ```csharp
    // Connect to the SLIM node — returns a connection ID used in later calls
    var connId = Slim.Connect("http://127.0.0.1:46357");

    Console.WriteLine($"Connected, connId={connId}");
    ```

=== "React Native"

    ```tsx
    // Create client config and connect — connect is synchronous in React Native
    const config = slimBindings.newInsecureClientConfig("http://192.168.1.x:46357");
    const connId = service.connect(config);

    console.log(`Connected, connId=${connId}`);
    ```

!!! note "TLS in Production"
    `new_insecure_client_config` skips TLS verification and is for development only. See [Authentication](../../architecture/authentication.md) for production TLS, mTLS, and JWT options.

## Putting It Together

=== "Python"

    ```python
    import asyncio
    import slim_bindings

    async def main():
        slim_bindings.uniffi_set_event_loop(asyncio.get_running_loop())
        slim_bindings.initialize_with_defaults()

        service = slim_bindings.get_global_service()

        client_config = slim_bindings.new_insecure_client_config("http://127.0.0.1:46357")
        conn_id = await service.connect_async(client_config)

        print(f"Connected, conn_id={conn_id}")
        # service and conn_id are passed to create_app in the next tutorial

    asyncio.run(main())
    ```

=== "Go"

    ```go
    package main

    import (
        "fmt"
        "log"

        slim "github.com/agntcy/slim-bindings-go"
    )

    func main() {
        slim.InitializeWithDefaults()

        config := slim.NewInsecureClientConfig("http://127.0.0.1:46357")
        connID, err := slim.GetGlobalService().ConnectAsync(config)
        if err != nil {
            log.Fatal(err)
        }

        fmt.Printf("Connected, connID=%d\n", connID)
        // service and connID are passed to create_app in the next tutorial
    }
    ```

=== "Java"

    ```java
    import io.agntcy.slim.bindings.*;

    public class Main {
        public static void main(String[] args) {
            SlimBindings.initializeWithDefaults();
            Service service = SlimBindings.getGlobalService();

            ClientConfig config = SlimBindings.newInsecureClientConfig("http://127.0.0.1:46357");
            Long connId = service.connect(config);

            System.out.println("Connected, connId=" + connId);
            // service and connId are passed to create_app in the next tutorial
        }
    }
    ```

=== "Kotlin"

    ```kotlin
    import io.agntcy.slim.bindings.*

    fun main() {
        initializeWithDefaults()
        val service = getGlobalService()

        val clientConfig = newInsecureClientConfig("http://127.0.0.1:46357")
        val connId: ULong = service.connectAsync(clientConfig)

        println("Connected, connId=$connId")
        // service and connId are passed to create_app in the next tutorial
    }
    ```

=== "Node.js"

    ```typescript
    import slimBindings from '@agntcy/slim-bindings';

    async function main() {
        slimBindings.initializeWithDefaults();
        const service = slimBindings.getGlobalService();

        const config = slimBindings.newInsecureClientConfig("http://127.0.0.1:46357");
        const connId = await service.connectAsync(config);

        console.log(`Connected, connId=${connId}`);
        // service and connId are passed to create_app in the next tutorial
    }

    main();
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim;

    Slim.Initialize();
    using var service = Slim.GetGlobalService();

    var connId = Slim.Connect("http://127.0.0.1:46357");

    Console.WriteLine($"Connected, connId={connId}");
    // service and connId are passed to CreateApp in the next tutorial
    ```

=== "React Native"

    ```tsx
    import slimBindings from '@agntcy/slim-bindings-react-native';

    await slimBindings.waitForJSIBindings(5000);
    slimBindings.initializeWithDefaults();
    const service = slimBindings.getGlobalService();

    const config = slimBindings.newInsecureClientConfig("http://192.168.1.x:46357");
    const connId = service.connect(config);

    console.log(`Connected, connId=${connId}`);
    // service and connId are passed to createApp in the next tutorial
    ```

## Next Steps

- [Creating an App](./tutorial-app.md) — Register an application identity using the `service` and `conn_id` from this tutorial
- [Authentication](../../architecture/authentication.md) — Secure connections with mTLS, JWT, or SPIRE
