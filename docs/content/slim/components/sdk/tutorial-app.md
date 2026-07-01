# Tutorial: Creating an App

This tutorial shows how to register an application identity in the SLIM network. An app identity is the routable name other services use to reach your application, and is required before establishing sessions or subscribing to channels.

## Prerequisites

- Completed [Connecting to SLIM](./tutorial-connect.md) — you need the `service` object and `conn_id` from that tutorial

## Application Identity

Every SLIM application is identified by a hierarchical name:

```
org/namespace/service/clientId
```

The first three components (`org/namespace/service`) are chosen by you and describe your organisation, deployment context, and service. The fourth component (`clientId`) is assigned by SLIM based on the client's cryptographic identity.

See [Naming](../../architecture/naming.md) for full details on the naming scheme.

## Step 1: Create the App

Pass the `service` obtained from initialisation to create an app bound to a name. The shared secret seeds the MLS cryptographic identity — use a long, random value in production.

=== "Python"

    ```python
    import slim_bindings

    # Define the application name (org / namespace / service)
    # clientId is derived from the cryptographic identity and assigned by SLIM
    local_name = slim_bindings.Name("myorg", "default", "my-service")

    # Create the app — registers this name with the cryptographic identity
    local_app = service.create_app_with_secret(local_name, "my-shared-secret")

    print(f"App created, id={local_app.id()}")
    ```

=== "Go"

    Refer to the [Go examples](https://github.com/agntcy/slim-bindings-go/tree/main/examples) in the slim-bindings-go repository.

=== "Kotlin"

    Refer to the [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples) in the slim-bindings repository.

=== ".NET"

    Refer to the [.NET examples](https://github.com/agntcy/slim-bindings/tree/main/dotnet/examples) in the slim-bindings repository.

## Step 2: Subscribe to Receive Messages

Subscribing tells the SLIM node to route inbound messages for this name to your application. Pass the `conn_id` returned by `connect_async` in the previous tutorial.

=== "Python"

    ```python
    # Subscribe — the SLIM node will now deliver messages for local_name to this app
    await local_app.subscribe_async(local_name, conn_id)

    print(f"Subscribed as: {local_name}")
    ```

=== "Go"

    Refer to the [Go examples](https://github.com/agntcy/slim-bindings-go/tree/main/examples).

=== "Kotlin"

    Refer to the [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples).

=== ".NET"

    Refer to the [.NET examples](https://github.com/agntcy/slim-bindings/tree/main/dotnet/examples).

## Step 3: Set a Route (Optional)

Before establishing a session to a remote application, your local SLIM node must know how to route messages to it. In most deployments this is managed automatically. For development or when running without a Controller, add the route manually:

=== "Python"

    ```python
    # Tell the local SLIM node how to reach the remote service
    remote_name = slim_bindings.Name("myorg", "default", "other-service")

    await local_app.set_route_async(remote_name, conn_id)
    ```

=== "Go"

    Refer to the [Go examples](https://github.com/agntcy/slim-bindings-go/tree/main/examples).

=== "Kotlin"

    Refer to the [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples).

=== ".NET"

    Refer to the [.NET examples](https://github.com/agntcy/slim-bindings/tree/main/dotnet/examples).

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

    local_name = slim_bindings.Name("myorg", "default", "my-service")
    local_app = service.create_app_with_secret(local_name, "my-shared-secret")
    await local_app.subscribe_async(local_name, conn_id)

    print(f"App ready: {local_name}, id={local_app.id()}")
    # local_app and conn_id are passed to create_session in the next tutorial

asyncio.run(main())
```

## Next Steps

- [Creating a Session](./tutorial-session.md) — Establish point-to-point and group sessions using the `local_app` and `conn_id` from this tutorial
- [Naming](../../architecture/naming.md) — Understand the full naming scheme including anycast vs. unicast
