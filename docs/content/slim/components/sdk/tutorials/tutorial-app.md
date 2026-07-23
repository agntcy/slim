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

See [Naming](../../../architecture/naming.md) for full details on the naming scheme.

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

    ```go
    // Define the application name (org / namespace / service)
    appName, err := slim.NameFromString("myorg/default/my-service")
    if err != nil {
        log.Fatal(err)
    }

    // Create the app — registers this name with the cryptographic identity
    app, err := slim.GetGlobalService().CreateAppWithSecret(appName, "my-shared-secret")
    if err != nil {
        log.Fatal(err)
    }
    defer app.Destroy()

    fmt.Printf("App created, id=%d\n", app.Id())
    ```

=== "Java"

    ```java
    // Define the application name (org / namespace / service)
    Name localName = Name.fromString("myorg/default/my-service");

    // Create the app — registers this name with the cryptographic identity
    App app = service.createAppWithSecret(localName, "my-shared-secret");

    System.out.println("App created, id=" + app.id());
    ```

=== "Kotlin"

    ```kotlin
    // Define the application name (org / namespace / service)
    val localName = Name.fromString("myorg/default/my-service")

    // Create the app — registers this name with the cryptographic identity
    val localApp = service.createAppWithSecret(localName, "my-shared-secret")

    println("App created, id=${localApp.id()}")
    ```

=== "Node.js"

    ```typescript
    // Define the application name (org / namespace / service)
    const localName = new slimBindings.Name("myorg", "default", "my-service");

    // Create the app — registers this name with the cryptographic identity
    const app = service.createAppWithSecret(localName, "my-shared-secret");

    console.log(`App created, id=${app.id()}`);
    ```

=== ".NET"

    ```csharp
    // Define the application name (org / namespace / service)
    // clientId is derived from the cryptographic identity and assigned by SLIM
    using var localName = SlimName.Parse("myorg/default/my-service");

    // Create the app — service is obtained from Slim.GetGlobalService() (see previous tutorial)
    var app = service.CreateApp(localName, "my-shared-secret");

    Console.WriteLine($"App created, id={app.Id}");
    ```

=== "React Native"

    ```tsx
    // Define the application name (org / namespace / service)
    // clientId is derived from the cryptographic identity and assigned by SLIM
    const localName = new slimBindings.Name("myorg", "default", "my-service");

    // Create the app — service is from the previous tutorial
    const app = service.createAppWithSecret(localName, "my-shared-secret");

    console.log(`App created, id=${app.id()}`);
    ```

=== "Rust"

    ```rust
    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::api::ProtoName;

    // Define the application name (org / namespace / service)
    // The client ID is assigned by SLIM based on the cryptographic identity
    let name = ProtoName::from_strings(["myorg", "default", "my-service"]);

    // Shared-secret identity — use a long, random value in production
    let provider = SharedSecret::new("myorg/default/my-service", "my-shared-secret")?;
    let verifier = SharedSecret::new("myorg/default/my-service", "my-shared-secret")?;

    // Create the app — returns the app handle and a notification receiver
    let (app, _rx) = service.create_app(&name, provider, verifier)?;
    println!("App created");
    ```

## Step 2: Subscribe to Receive Messages

Subscribing tells the SLIM node to route inbound messages for this name to your application. Pass the `conn_id` returned by `connect_async` in the previous tutorial.

=== "Python"

    ```python
    # Subscribe — the SLIM node will now deliver messages for local_name to this app
    await local_app.subscribe_async(local_name, conn_id)

    print(f"Subscribed as: {local_name}")
    ```

=== "Go"

    ```go
    // Subscribe — the SLIM node will now deliver messages for appName to this app
    if err := app.SubscribeAsync(app.Name(), &connID); err != nil {
        log.Fatal(err)
    }

    fmt.Println("Subscribed as:", appName)
    ```

=== "Java"

    ```java
    // Subscribe — the SLIM node will now deliver messages for localName to this app
    app.subscribe(app.name(), connId);

    System.out.println("Subscribed as: " + localName);
    ```

=== "Kotlin"

    ```kotlin
    // Subscribe — the SLIM node will now deliver messages for localName to this app
    localApp.subscribeAsync(localName, connId)

    println("Subscribed as: $localName")
    ```

=== "Node.js"

    ```typescript
    // Subscribe — the SLIM node will now deliver messages for localName to this app
    await app.subscribeAsync(localName, BigInt(connId));

    console.log(`Subscribed as: ${localName}`);
    ```

=== ".NET"

    ```csharp
    // Subscribe — the SLIM node will now deliver messages for localName to this app
    app.Subscribe(app.Name, connId);

    Console.WriteLine($"Subscribed as: {app.Name}");
    ```

=== "React Native"

    ```tsx
    // Subscribe — the SLIM node will now deliver messages for localName to this app
    await app.subscribeAsync(localName, connId);

    console.log(`Subscribed as: ${localName}`);
    ```

=== "Rust"

    ```rust
    // Subscribe — the SLIM node will now deliver messages for name to this app
    app.subscribe(&name, Some(conn_id)).await?;
    println!("Subscribed as: myorg/default/my-service");
    ```

## Step 3: Set a Route (Optional)

Before establishing a session to a remote application, your local SLIM node must know how to route messages to it. In most deployments this is managed automatically. For development or when running without a Controller, add the route manually:

=== "Python"

    ```python
    # Tell the local SLIM node how to reach the remote service
    remote_name = slim_bindings.Name("myorg", "default", "other-service")

    await local_app.set_route_async(remote_name, conn_id)
    ```

=== "Go"

    ```go
    // Tell the local SLIM node how to reach the remote service
    remoteName, _ := slim.NameFromString("myorg/default/other-service")

    if err := app.SetRouteAsync(remoteName, connID); err != nil {
        log.Fatal(err)
    }
    ```

=== "Java"

    ```java
    // Tell the local SLIM node how to reach the remote service
    Name remoteName = Name.fromString("myorg/default/other-service");

    app.setRoute(remoteName, connId);
    ```

=== "Kotlin"

    ```kotlin
    // Tell the local SLIM node how to reach the remote service
    val remoteName = Name.fromString("myorg/default/other-service")

    localApp.setRouteAsync(remoteName, connId)
    ```

=== "Node.js"

    ```typescript
    // Tell the local SLIM node how to reach the remote service
    const remoteName = new slimBindings.Name("myorg", "default", "other-service");

    app.setRoute(remoteName, Number(connId));
    ```

=== ".NET"

    ```csharp
    // Tell the local SLIM node how to reach the remote service
    using var remoteName = SlimName.Parse("myorg/default/other-service");
    app.SetRoute(remoteName, connId);
    ```

=== "React Native"

    ```tsx
    // Tell the local SLIM node how to reach the remote service
    const remoteName = new slimBindings.Name("myorg", "default", "other-service");
    await app.setRoute(remoteName, connId);
    ```

=== "Rust"

    ```rust
    // Tell the local SLIM node how to reach the remote service
    let remote_name = ProtoName::from_strings(["myorg", "default", "other-service"]);
    app.set_route(&remote_name, conn_id).await?;
    ```

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

        local_name = slim_bindings.Name("myorg", "default", "my-service")
        local_app = service.create_app_with_secret(local_name, "my-shared-secret")
        await local_app.subscribe_async(local_name, conn_id)

        print(f"App ready: {local_name}, id={local_app.id()}")
        # local_app and conn_id are passed to create_session in the next tutorial

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

        appName, err := slim.NameFromString("myorg/default/my-service")
        if err != nil {
            log.Fatal(err)
        }

        app, err := slim.GetGlobalService().CreateAppWithSecret(appName, "my-shared-secret")
        if err != nil {
            log.Fatal(err)
        }
        defer app.Destroy()

        if err := app.SubscribeAsync(app.Name(), &connID); err != nil {
            log.Fatal(err)
        }

        fmt.Printf("App ready: %s, id=%d\n", appName, app.Id())
        // app and connID are passed to create_session in the next tutorial
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

            Name localName = Name.fromString("myorg/default/my-service");
            App app = service.createAppWithSecret(localName, "my-shared-secret");
            app.subscribe(app.name(), connId);

            System.out.println("App ready: " + localName + ", id=" + app.id());
            // app and connId are passed to create_session in the next tutorial
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

        val localName = Name.fromString("myorg/default/my-service")
        val localApp = service.createAppWithSecret(localName, "my-shared-secret")
        localApp.subscribeAsync(localName, connId)

        println("App ready: $localName, id=${localApp.id()}")
        // localApp and connId are passed to create_session in the next tutorial
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

        const localName = new slimBindings.Name("myorg", "default", "my-service");
        const app = service.createAppWithSecret(localName, "my-shared-secret");
        await app.subscribeAsync(localName, BigInt(connId));

        console.log(`App ready: ${localName}, id=${app.id()}`);
        // app and connId are passed to create_session in the next tutorial
    }

    main();
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim;

    Slim.Initialize();

    var connId = Slim.Connect("http://127.0.0.1:46357");

    using var localName = SlimName.Parse("myorg/default/my-service");
    using var service = Slim.GetGlobalService();
    var app = service.CreateApp(localName, "my-shared-secret");
    app.Subscribe(app.Name, connId);

    Console.WriteLine($"App ready: {app.Name}, id={app.Id}");
    // app and connId are passed to CreateSession in the next tutorial
    ```

=== "React Native"

    ```tsx
    import slimBindings from '@agntcy/slim-bindings-react-native';

    await slimBindings.waitForJSIBindings(5000);
    slimBindings.initializeWithDefaults();
    const service = slimBindings.getGlobalService();

    const config = slimBindings.newInsecureClientConfig("http://192.168.1.x:46357");
    const connId = service.connect(config);

    const localName = new slimBindings.Name("myorg", "default", "my-service");
    const app = service.createAppWithSecret(localName, "my-shared-secret");
    await app.subscribeAsync(localName, connId);

    console.log(`App ready: ${localName}, id=${app.id()}`);
    // app and connId are passed to createSession in the next tutorial
    ```

=== "Rust"

    ```rust
    use slim_service::ServiceConfiguration;
    use slim_service::config::{ClientConfig, TlsClientConfig};
    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::api::{ID, ProtoName};

    #[tokio::main]
    async fn main() -> anyhow::Result<()> {
        let mut config = ServiceConfiguration::new();
        config.with_dataplane_client(vec![
            ClientConfig::with_endpoint("http://127.0.0.1:46357")
                .with_tls_setting(TlsClientConfig::default().with_insecure(true)),
        ]);
        let service = config.build_server(ID::new_with_str("slim/0")?)?;
        service.run().await?;
        let conn_id = service.get_connection_id("http://127.0.0.1:46357").unwrap();

        let name = ProtoName::from_strings(["myorg", "default", "my-service"]);
        let provider = SharedSecret::new("myorg/default/my-service", "my-shared-secret")?;
        let verifier = SharedSecret::new("myorg/default/my-service", "my-shared-secret")?;

        let (app, _rx) = service.create_app(&name, provider, verifier)?;
        app.subscribe(&name, Some(conn_id)).await?;

        println!("App ready: myorg/default/my-service");
        // app and conn_id are passed to create_session in the next tutorial

        Ok(())
    }
    ```

## Next Steps

- [Creating a Session](./tutorial-session.md) — Establish point-to-point and group sessions using the `local_app` and `conn_id` from this tutorial
- [Naming](../../../architecture/naming.md) — Understand the full naming scheme including anycast vs. unicast
