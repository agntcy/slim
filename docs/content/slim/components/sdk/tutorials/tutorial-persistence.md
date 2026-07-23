# Tutorial: Session Persistence and Restore

This tutorial shows how to persist group session state so that an application can restart and resume its sessions without repeating the invite and MLS key-exchange handshake. It also covers the close and rejoin lifecycle for participants that need to go offline temporarily and come back.

## Prerequisites

- Completed [Creating a Session](./tutorial-session.md) — you need a running group session with at least one invited participant
- A writable directory on disk for the session store (e.g. `./slim-state/`)

For conceptual background see [Session State Persistence](../../../architecture/sessions/group.md#session-state-persistence) and [Participant Liveness and Disconnection Detection](../../../architecture/sessions/group.md#participant-liveness-and-disconnection-detection).

## Step 1: Create an App with Persistence Enabled

Instead of `create_app_with_secret`, use `create_app_with_persistence`. Pass a `PersistenceConfig` specifying the storage directory and a passphrase to encrypt the state at rest.

!!! warning "Passphrase"
    Always set a passphrase in production. Without one, the store is authenticated but not confidential — anyone who can read the database file and knows the app name can decrypt it.

=== "Python"

    ```python
    import asyncio
    import slim_bindings

    async def main():
        slim_bindings.uniffi_set_event_loop(asyncio.get_running_loop())
        slim_bindings.initialize_with_defaults()

        service = slim_bindings.get_global_service()
        conn_id = await service.connect_async(
            slim_bindings.new_insecure_client_config("http://127.0.0.1:46357")
        )

        local_name = slim_bindings.Name("myorg", "default", "my-service")

        persistence = slim_bindings.PersistenceConfig(
            path="./slim-state",
            passphrase="change-me-in-production",
        )

        provider = slim_bindings.IdentityProviderConfig.SHARED_SECRET(
            id=str(local_name), data="my-shared-secret"
        )
        verifier = slim_bindings.IdentityVerifierConfig.SHARED_SECRET(
            id=str(local_name), data="my-shared-secret"
        )

        app = await service.create_app_with_persistence_async(
            local_name,
            provider,
            verifier,
            slim_bindings.Direction.BIDIRECTIONAL,
            persistence,
        )
        await app.subscribe_async(local_name, conn_id)
        print(f"App ready with persistence: {local_name}")
        return app, conn_id
    ```

=== "Go"

    ```go
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

        appName, _ := slim.NameFromString("myorg/default/my-service")
        passphrase := "change-me-in-production"

        persistence := slim.PersistenceConfig{
            Path:       "./slim-state",
            Passphrase: &passphrase,
        }
        provider := slim.IdentityProviderConfigSharedSecret{
            Id:   appName.String(),
            Data: "my-shared-secret",
        }
        verifier := slim.IdentityVerifierConfigSharedSecret{
            Id:   appName.String(),
            Data: "my-shared-secret",
        }

        app, err := slim.GetGlobalService().CreateAppWithPersistence(
            appName,
            provider,
            verifier,
            slim.DirectionBidirectional,
            persistence,
        )
        if err != nil {
            log.Fatal(err)
        }
        defer app.Destroy()

        if err := app.SubscribeAsync(app.Name(), &connID); err != nil {
            log.Fatal(err)
        }
        fmt.Println("App ready with persistence:", appName)
    }
    ```

=== "Java"

    ```java
    import io.agntcy.slim.bindings.*;

    SlimBindings.initializeWithDefaults();
    Service service = SlimBindings.getGlobalService();

    ClientConfig config = SlimBindings.newInsecureClientConfig("http://127.0.0.1:46357");
    Long connId = service.connect(config);

    Name localName = Name.fromString("myorg/default/my-service");

    PersistenceConfig persistence = new PersistenceConfig(
        "./slim-state",
        "change-me-in-production"
    );
    IdentityProviderConfig provider = IdentityProviderConfig.sharedSecret(
        localName.toString(), "my-shared-secret"
    );
    IdentityVerifierConfig verifier = IdentityVerifierConfig.sharedSecret(
        localName.toString(), "my-shared-secret"
    );

    App app = service.createAppWithPersistence(
        localName, provider, verifier,
        Direction.BIDIRECTIONAL, persistence
    );
    app.subscribe(app.name(), connId);

    System.out.println("App ready with persistence: " + localName);
    ```

=== "Kotlin"

    ```kotlin
    import io.agntcy.slim.bindings.*

    initializeWithDefaults()
    val service = getGlobalService()

    val connId: ULong = service.connectAsync(newInsecureClientConfig("http://127.0.0.1:46357"))

    val localName = Name.fromString("myorg/default/my-service")

    val persistence = PersistenceConfig(
        path = "./slim-state",
        passphrase = "change-me-in-production"
    )
    val provider = IdentityProviderConfig.SharedSecret(
        id = localName.toString(), data = "my-shared-secret"
    )
    val verifier = IdentityVerifierConfig.SharedSecret(
        id = localName.toString(), data = "my-shared-secret"
    )

    val app = service.createAppWithPersistence(
        localName, provider, verifier,
        Direction.BIDIRECTIONAL, persistence
    )
    app.subscribeAsync(localName, connId)

    println("App ready with persistence: $localName")
    ```

=== "Node.js"

    ```typescript
    import slimBindings from '@agntcy/slim-bindings';

    slimBindings.initializeWithDefaults();
    const service = slimBindings.getGlobalService();

    const connId = await service.connectAsync(
        slimBindings.newInsecureClientConfig("http://127.0.0.1:46357")
    );

    const localName = new slimBindings.Name("myorg", "default", "my-service");

    const persistence = {
        path: "./slim-state",
        passphrase: "change-me-in-production",
    };
    const provider = { sharedSecret: { id: localName.toString(), data: "my-shared-secret" } };
    const verifier = { sharedSecret: { id: localName.toString(), data: "my-shared-secret" } };

    const app = await service.createAppWithPersistenceAsync(
        localName, provider, verifier,
        "bidirectional", persistence
    );
    await app.subscribeAsync(localName, BigInt(connId));

    console.log(`App ready with persistence: ${localName}`);
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim;

    Slim.Initialize();

    var connId = Slim.Connect("http://127.0.0.1:46357");

    using var localName = SlimName.Parse("myorg/default/my-service");
    using var service = Slim.GetGlobalService();

    var persistence = new SlimPersistenceConfig(
        path: "./slim-state",
        passphrase: "change-me-in-production"
    );
    var provider = SlimIdentityProviderConfig.SharedSecret(localName.ToString(), "my-shared-secret");
    var verifier = SlimIdentityVerifierConfig.SharedSecret(localName.ToString(), "my-shared-secret");

    var app = await service.CreateAppWithPersistenceAsync(
        localName, provider, verifier,
        SlimDirection.Bidirectional, persistence
    );
    app.Subscribe(app.Name, connId);

    Console.WriteLine($"App ready with persistence: {localName}");
    ```

=== "Rust"

    ```rust
    use slim_service::ServiceConfiguration;
    use slim_service::config::{ClientConfig, TlsClientConfig};
    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::api::{ID, ProtoName};
    use slim_persistence::PersistenceConfig;
    use slim_session::Direction;

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

        let (app, _rx) = service.create_app_with_direction_and_persistence(
            &name,
            provider,
            verifier,
            Direction::Bidirectional,
            Some(PersistenceConfig::new("./slim-state")),
        )?;
        app.subscribe(&name, Some(conn_id)).await?;
        println!("App ready with persistence: myorg/default/my-service");

        Ok(())
    }
    ```

## Step 2: Use the Session Normally

Create a group session and exchange messages exactly as shown in [Creating a Session](./tutorial-session.md). The session layer silently checkpoints MLS state and membership to the store as the session progresses — no additional calls required.

## Step 3: Restore Sessions After a Restart

On the next startup, create a new app using the **same name, secret, store path, and passphrase**, then call `restore_sessions`. The session layer reads the persisted state, re-establishes routing, and rejoins the MLS group without repeating the full handshake.

=== "Python"

    ```python
    # After restart — same name, secret, path, and passphrase as before
    app = await service.create_app_with_persistence_async(
        local_name, provider, verifier,
        slim_bindings.Direction.BIDIRECTIONAL, persistence,
    )
    await app.subscribe_async(local_name, conn_id)

    # Restore all previously active sessions
    sessions = await app.restore_sessions_async(conn_id)
    print(f"Restored {len(sessions)} session(s)")

    # Each restored session is immediately usable
    for session in sessions:
        await session.publish_async(b"back online", None, None)
    ```

=== "Go"

    ```go
    // After restart — same name, secret, path, and passphrase as before
    app, err = slim.GetGlobalService().CreateAppWithPersistence(
        appName, provider, verifier,
        slim.DirectionBidirectional, persistence,
    )
    if err != nil {
        log.Fatal(err)
    }
    defer app.Destroy()

    if err := app.SubscribeAsync(app.Name(), &connID); err != nil {
        log.Fatal(err)
    }

    // Restore all previously active sessions
    sessions, err := app.RestoreSessions(connID)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("Restored %d session(s)\n", len(sessions))

    // Each restored session is immediately usable
    for _, session := range sessions {
        if err := session.PublishAndWaitAsync([]byte("back online"), nil, nil); err != nil {
            log.Println("publish error:", err)
        }
    }
    ```

=== "Java"

    ```java
    // After restart — same name, secret, path, and passphrase as before
    App app = service.createAppWithPersistence(
        localName, provider, verifier,
        Direction.BIDIRECTIONAL, persistence
    );
    app.subscribe(app.name(), connId);

    // Restore all previously active sessions
    List<Session> sessions = app.restoreSessions(connId);
    System.out.println("Restored " + sessions.size() + " session(s)");

    // Each restored session is immediately usable
    for (Session session : sessions) {
        session.publishAndWait("back online".getBytes(), null, null);
    }
    ```

=== "Kotlin"

    ```kotlin
    // After restart — same name, secret, path, and passphrase as before
    val app = service.createAppWithPersistence(
        localName, provider, verifier,
        Direction.BIDIRECTIONAL, persistence
    )
    app.subscribeAsync(localName, connId)

    // Restore all previously active sessions
    val sessions = app.restoreSessionsAsync(connId)
    println("Restored ${sessions.size} session(s)")

    // Each restored session is immediately usable
    for (session in sessions) {
        session.publishAsync("back online".toByteArray(), null, null)
    }
    ```

=== "Node.js"

    ```typescript
    // After restart — same name, secret, path, and passphrase as before
    const app = await service.createAppWithPersistenceAsync(
        localName, provider, verifier, "bidirectional", persistence
    );
    await app.subscribeAsync(localName, BigInt(connId));

    // Restore all previously active sessions
    const sessions = await app.restoreSessionsAsync(connId);
    console.log(`Restored ${sessions.length} session(s)`);

    // Each restored session is immediately usable
    for (const session of sessions) {
        await session.publishAndWaitAsync(Buffer.from("back online"), undefined, undefined);
    }
    ```

=== ".NET"

    ```csharp
    // After restart — same name, secret, path, and passphrase as before
    var app = await service.CreateAppWithPersistenceAsync(
        localName, provider, verifier,
        SlimDirection.Bidirectional, persistence
    );
    app.Subscribe(app.Name, connId);

    // Restore all previously active sessions
    var sessions = await app.RestoreSessionsAsync(connId);
    Console.WriteLine($"Restored {sessions.Count} session(s)");

    // Each restored session is immediately usable
    foreach (var session in sessions)
    {
        await session.PublishAsync("back online");
    }
    ```

=== "Rust"

    ```rust
    // After restart — same name, secret, and store path as before
    let (app, _rx) = service.create_app_with_direction_and_persistence(
        &name,
        provider,
        verifier,
        Direction::Bidirectional,
        Some(PersistenceConfig::new("./slim-state")),
    )?;
    app.subscribe(&name, Some(conn_id)).await?;

    // Restore all previously active sessions
    let sessions = app.restore_sessions(conn_id).await?;
    println!("Restored {} session(s)", sessions.len());

    // Each restored session is immediately usable
    for session in &sessions {
        session.publish(&channel_name, b"back online".to_vec(), None, None).await?;
    }
    ```

## Close and Rejoin (Coming Soon)

!!! note "Not yet available in SDK bindings"
    The `close` and `rejoin` operations — which let a participant explicitly go offline and return without a restart — exist in the Rust session layer but are not yet surfaced in the SDK language bindings. They will be added in a future release.

    Once available, the flow will be:

    1. Call `session.close()` to broadcast an `OFFLINE` state update and pause participation. Other group members stop expecting acknowledgements from this participant and rotate them out of MLS key material.
    2. Call `session.rejoin()` to broadcast an `ONLINE` state update and re-enter the group. The session layer performs an MLS re-key to include the participant in new key material.

    Until then, use the restart-and-restore flow described above for any scenario requiring a participant to temporarily leave a session.

## Next Steps

- [Groups](../../../architecture/sessions/group.md) — Participant liveness, close/rejoin, and persistence concepts in detail
- [Creating a Session](./tutorial-session.md) — Group session creation and the invite lifecycle
- [Receiving a Session](./tutorial-receive.md) — Listening for incoming sessions on a restored app
