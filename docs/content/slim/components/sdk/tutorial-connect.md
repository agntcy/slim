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

    Refer to the [Go examples](https://github.com/agntcy/slim-bindings-go/tree/main/examples) in the slim-bindings-go repository for the equivalent initialisation pattern.

=== "Kotlin"

    Refer to the [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples) in the slim-bindings repository.

=== ".NET"

    Refer to the [.NET examples](https://github.com/agntcy/slim-bindings/tree/main/dotnet/examples) in the slim-bindings repository.

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

    Refer to the [Go examples](https://github.com/agntcy/slim-bindings-go/tree/main/examples) in the slim-bindings-go repository.

=== "Kotlin"

    Refer to the [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples).

=== ".NET"

    Refer to the [.NET examples](https://github.com/agntcy/slim-bindings/tree/main/dotnet/examples).

!!! note "TLS in Production"
    `new_insecure_client_config` skips TLS verification and is for development only. See [Authentication](../../architecture/authentication.md) for production TLS, mTLS, and JWT options.

## Putting It Together

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

## Next Steps

- [Creating an App](./tutorial-app.md) — Register an application identity using the `service` and `conn_id` from this tutorial
- [Authentication](../../architecture/authentication.md) — Secure connections with mTLS, JWT, or SPIRE
