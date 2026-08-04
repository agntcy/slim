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

=== "Rust"

    ```rust
    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::api::ProtoName;

    // Define the application name (org / namespace / service)
    // The client ID is assigned by SLIM based on the cryptographic identity
    let name = ProtoName::from_strings(["myorg", "default", "my-service"]);

    // Shared-secret identity — use a long, random value in production
    let provider = SharedSecret::new("myorg/default/my-service", "change-me-before-going-to-production")?;
    let verifier = SharedSecret::new("myorg/default/my-service", "change-me-before-going-to-production")?;

    // Create the app — returns the app handle and a notification receiver
    let (app, _rx) = service.create_app(&name, provider, verifier)?;
    println!("App created");
    ```

=== "Python"

    ```python
    import slim_bindings

    # Define the application name (org / namespace / service)
    # clientId is derived from the cryptographic identity and assigned by SLIM
    local_name = slim_bindings.Name("myorg", "default", "my-service")

    # Create the app — registers this name with the cryptographic identity
    local_app = service.create_app_with_secret(local_name, "change-me-before-going-to-production")

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
    app, err := slim.GetGlobalService().CreateAppWithSecret(appName, "change-me-before-going-to-production")
    if err != nil {
        log.Fatal(err)
    }
    defer app.Destroy()

    fmt.Printf("App created, id=%s\n", app.Id())
    ```

=== "Java"

    ```java
    // Define the application name (org / namespace / service)
    Name localName = Name.fromString("myorg/default/my-service");

    // Create the app — registers this name with the cryptographic identity
    App app = service.createAppWithSecret(localName, "change-me-before-going-to-production");

    System.out.println("App created, id=" + app.id());
    ```

=== "Kotlin"

    ```kotlin
    // Define the application name (org / namespace / service)
    val localName = Name.fromString("myorg/default/my-service")

    // Create the app — registers this name with the cryptographic identity
    val localApp = service.createAppWithSecret(localName, "change-me-before-going-to-production")

    println("App created, id=${localApp.id()}")
    ```

=== "Node.js"

    ```typescript
    // Define the application name (org / namespace / service)
    const localName = new slimBindings.Name("myorg", "default", "my-service");

    // Create the app — registers this name with the cryptographic identity
    const app = service.createAppWithSecret(localName, "change-me-before-going-to-production");

    console.log(`App created, id=${app.id()}`);
    ```

=== ".NET"

    ```csharp
    // Define the application name (org / namespace / service)
    // clientId is derived from the cryptographic identity and assigned by SLIM
    using var localName = SlimName.Parse("myorg/default/my-service");

    // Create the app — service is obtained from Slim.GetGlobalService() (see previous tutorial)
    var app = service.CreateApp(localName, "change-me-before-going-to-production");

    Console.WriteLine($"App created, id={app.Id}");
    ```

=== "React Native"

    ```tsx
    // Define the application name (org / namespace / service)
    // clientId is derived from the cryptographic identity and assigned by SLIM
    const localName = new slimBindings.Name("myorg", "default", "my-service");

    // Create the app — service is from the previous tutorial
    const app = service.createAppWithSecret(localName, "change-me-before-going-to-production");

    console.log(`App created, id=${app.id()}`);
    ```

## Step 2: Subscribe to Receive Messages

Subscribing tells the SLIM node to route inbound messages for this name to your application. Pass the `conn_id` returned by `connect_async` in the previous tutorial.

=== "Rust"

    ```rust
    // Subscribe — the SLIM node will now deliver messages for name to this app
    app.subscribe(&name, Some(conn_id)).await?;
    println!("Subscribed as: myorg/default/my-service");
    ```

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

## Step 3: Set a Route (Optional)

Before establishing a session to a remote application, your local SLIM node must know how to route messages to it. In most deployments this is managed automatically. For development or when running without a Controller, add the route manually:

=== "Rust"

    ```rust
    // Tell the local SLIM node how to reach the remote service
    let remote_name = ProtoName::from_strings(["myorg", "default", "other-service"]);
    app.set_route(&remote_name, conn_id).await?;
    ```

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

    app.setRoute(remoteName, connId);
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

## Putting It Together

{% include-markdown "slim/components/sdk/tutorials/_snippets/putting-it-together.md" %}

## Next Steps

- [Creating a Session](./tutorial-session.md) — Establish point-to-point and group sessions using the `local_app` and `conn_id` from this tutorial
- [Naming](../../../architecture/naming.md) — Understand the full naming scheme including anycast vs. unicast
