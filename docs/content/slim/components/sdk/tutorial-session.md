# Tutorial: Creating a Session

This tutorial shows how to create point-to-point and group sessions to exchange messages between applications. Sessions provide reliable, optionally encrypted communication on top of the SLIM data plane.

## Prerequisites

- Completed [Creating an App](./tutorial-app.md)
- Two running SLIM applications or two instances of the same service

For conceptual background on session types, see [Sessions](../../architecture/sessions/index.md).

## Point-to-Point Session

A point-to-point session connects your application to a single remote instance. SLIM performs a discovery phase to locate the remote, then binds all subsequent messages in the session to that endpoint.

### Create the Session

=== "Python"

    ```python
    import asyncio
    import datetime
    import slim_bindings

    async def run_client(app, remote_name):
        # Configure the session
        session_config = slim_bindings.SessionConfig(
            session_type=slim_bindings.SessionType.POINT_TO_POINT,
            enable_mls=False,       # Set True to enable end-to-end encryption
            max_retries=5,
            interval=datetime.timedelta(seconds=5),
        )

        # Create the session — discovery happens automatically
        session_context = await app.create_session_async(session_config, remote_name)

        # Wait until the session is fully established
        await session_context.completion.wait_async()

        session = session_context.session
        print("Point-to-point session established")
        return session
    ```

=== "Go"

    ```go
    import (
        "fmt"
        "log"

        slim "github.com/agntcy/slim-bindings-go"
    )

    func runClient(app *slim.App, remoteName slim.Name) *slim.Session {
        // Configure the session
        config := slim.SessionConfig{
            SessionType: slim.SessionTypePointToPoint,
            MlsSettings: nil, // Set to &slim.MlsSettings{...} to enable E2E encryption
        }

        // Create the session — discovery happens automatically
        // Blocks until the session is fully established
        session, err := app.CreateSessionAndWaitAsync(config, remoteName)
        if err != nil {
            log.Fatal(err)
        }

        fmt.Println("Point-to-point session established")
        return session
    }
    ```

=== "Java"

    ```java
    import io.agntcy.slim.bindings.*;
    import java.time.Duration;
    import java.util.Map;

    Session runClient(App app, Name remoteName) {
        // Configure the session
        SessionConfig sessionConfig = new SessionConfig(
            SessionType.POINT_TO_POINT,
            5,                       // maxRetries
            Duration.ofSeconds(5),   // interval
            Map.of(),                // metadata
            null                     // mlsSettings (set to new MlsSettings(100) for E2E encryption)
        );

        // Create the session — discovery happens automatically
        // Blocks until the session is fully established
        Session session = app.createSessionAndWait(sessionConfig, remoteName);

        System.out.println("Point-to-point session established");
        return session;
    }
    ```

=== "Kotlin"

    ```kotlin
    import io.agntcy.slim.bindings.*
    import java.time.Duration

    suspend fun runClient(app: App, remoteName: Name): Session {
        // Configure the session
        val sessionConfig = SessionConfig(
            sessionType = SessionType.POINT_TO_POINT,
            maxRetries = 5u,
            interval = Duration.ofSeconds(5),
            metadata = emptyMap(),
            mlsSettings = null // Set to MlsSettings(100u) for E2E encryption
        )

        // Create the session — discovery happens automatically
        val sessionContext = app.createSession(sessionConfig, remoteName)

        // Wait until the session is fully established
        sessionContext.completion.waitAsync()

        val session = sessionContext.session
        println("Point-to-point session established")
        return session
    }
    ```

=== "Node.js"

    ```typescript
    import slimBindings from '@agntcy/slim-bindings';

    async function runClient(app, remoteName) {
        // Configure the session
        const sessionConfig = {
            sessionType: "pointToPoint" as const,
            enableMls: false, // Set true to enable end-to-end encryption
            maxRetries: 5,
            interval: 5000, // milliseconds
            metadata: new Map()
        };

        // Create the session — discovery happens automatically
        // Resolves when the session is fully established
        const session = await app.createSessionAndWaitAsync(sessionConfig, remoteName);

        console.log("Point-to-point session established");
        return session;
    }
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim;

    // Configure the session
    var config = new SlimSessionConfig
    {
        SessionType = SlimSessionType.PointToPoint,
        MlsSettings = null  // Set to new SlimMlsSettings() to enable E2E encryption
    };

    // Create the session — discovery happens automatically
    // Blocks until the session is fully established
    using var session = await app.CreateSessionAsync(remoteName, config);

    Console.WriteLine("Point-to-point session established");
    ```

=== "React Native"

    ```tsx
    // Configure the session — SessionType is an enum, metadata must be a Map
    const sessionConfig = {
        sessionType: slimBindings.SessionType.PointToPoint,
        enableMls: false,  // set true to enable E2E encryption
        metadata: new Map()
    };

    // Create the session — discovery happens automatically
    const session = await app.createSessionAndWaitAsync(sessionConfig, remoteName);

    console.log("Point-to-point session established");
    ```

### Send and Receive Messages

=== "Python"

    ```python
    # Send a message
    await session.publish_async(
        b"hello",   # payload: bytes
        None,       # payload_type: str | None
        None,       # metadata: dict | None
    )

    # Receive a reply
    received = await session.get_message_async(
        timeout=datetime.timedelta(seconds=30)
    )
    print("Received:", received.payload.decode())

    # Echo the message back
    await session.publish_async(received.payload, None, None)
    ```

=== "Go"

    ```go
    import "time"

    // Send a message
    if err := session.PublishAndWaitAsync([]byte("hello"), nil, nil); err != nil {
        log.Fatal(err)
    }

    // Receive a reply
    timeout := 30 * time.Second
    msg, err := session.GetMessageAsync(&timeout)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Println("Received:", string(msg.Payload))

    // Echo the message back
    if err := session.PublishAndWaitAsync(msg.Payload, nil, nil); err != nil {
        log.Fatal(err)
    }
    ```

=== "Java"

    ```java
    import java.time.Duration;

    // Send a message
    session.publishAndWait("hello".getBytes(), null, null);

    // Receive a reply
    ReceivedMessage msg = session.getMessage(Duration.ofSeconds(30));
    System.out.println("Received: " + new String(msg.payload()));

    // Echo the message back
    session.publishAndWait(msg.payload(), null, null);
    ```

=== "Kotlin"

    ```kotlin
    import java.time.Duration

    // Send a message
    session.publishAsync("hello".toByteArray(), null, null)

    // Receive a reply
    val msg = session.getMessageAsync(Duration.ofSeconds(30))
    println("Received: " + String(msg.payload))

    // Echo the message back
    session.publishAsync(msg.payload, null, null)
    ```

=== "Node.js"

    ```typescript
    // Send a message
    await session.publishAndWaitAsync(Buffer.from("hello"), undefined, undefined);

    // Receive a reply
    const msg = await session.getMessageAsync(30000); // timeout in milliseconds
    console.log("Received:", Buffer.from(msg.payload).toString());

    // Echo the message back
    await session.publishAndWaitAsync(msg.payload, undefined, undefined);
    ```

=== ".NET"

    ```csharp
    // Send a message
    await session.PublishAsync("hello");

    // Receive a reply
    var msg = await session.GetMessageAsync(TimeSpan.FromSeconds(30));
    Console.WriteLine($"Received: {msg.Text}");

    // Echo the message back
    await session.ReplyAsync(msg, msg.Text);
    ```

=== "React Native"

    ```tsx
    // Send a message — payload is Uint8Array in React Native
    const payload = new Uint8Array("hello".split('').map(c => c.charCodeAt(0)));
    await session.publishAndWaitAsync(payload, undefined, undefined);

    // Receive a reply (timeout in milliseconds)
    const msg = await session.getMessageAsync(30000);
    const text = String.fromCharCode(...new Uint8Array(msg.payload));
    console.log("Received:", text);

    // Echo the message back
    await session.publishToAndWaitAsync(msg.context, msg.payload, undefined, undefined);
    ```

## Group Session

A group session enables many-to-many communication on a named channel. Every message sent to the channel is delivered to all current participants.

### Create the Session

=== "Python"

    ```python
    async def create_group_session(app, channel_name):
        session_config = slim_bindings.SessionConfig(
            session_type=slim_bindings.SessionType.GROUP,
            enable_mls=False,
            max_retries=5,
            interval=datetime.timedelta(seconds=5),
        )

        # Create the session on the given channel
        session_context = app.create_session(session_config, channel_name)

        # Wait until the session is ready
        await session_context.completion.wait_async()

        session = session_context.session
        print("Group session created on channel:", channel_name)
        return session
    ```

=== "Go"

    ```go
    func createGroupSession(app *slim.App, channelName slim.Name) *slim.Session {
        config := slim.SessionConfig{
            SessionType: slim.SessionTypeGroup,
            MlsSettings: nil,
        }

        // Create the session on the given channel
        // Blocks until the session is ready
        session, err := app.CreateSessionAndWaitAsync(config, channelName)
        if err != nil {
            log.Fatal(err)
        }

        fmt.Println("Group session created on channel:", channelName)
        return session
    }
    ```

=== "Java"

    ```java
    Session createGroupSession(App app, Name channelName) {
        SessionConfig sessionConfig = new SessionConfig(
            SessionType.GROUP,
            5,                       // maxRetries
            Duration.ofSeconds(5),   // interval
            Map.of(),                // metadata
            null                     // mlsSettings
        );

        // Create the session on the given channel
        // Blocks until the session is ready
        Session session = app.createSessionAndWait(sessionConfig, channelName);

        System.out.println("Group session created on channel: " + channelName);
        return session;
    }
    ```

=== "Kotlin"

    ```kotlin
    suspend fun createGroupSession(app: App, channelName: Name): Session {
        val sessionConfig = SessionConfig(
            sessionType = SessionType.GROUP,
            maxRetries = 5u,
            interval = Duration.ofSeconds(5),
            metadata = emptyMap(),
            mlsSettings = null
        )

        // Create the session on the given channel
        val sessionContext = app.createSession(sessionConfig, channelName)

        // Wait until the session is ready
        sessionContext.completion.waitAsync()

        val session = sessionContext.session
        println("Group session created on channel: $channelName")
        return session
    }
    ```

=== "Node.js"

    Group sessions are not yet supported in the Node.js bindings.

=== ".NET"

    ```csharp
    using Agntcy.Slim;

    var config = new SlimSessionConfig
    {
        SessionType = SlimSessionType.Group,
        MlsSettings = null,  // Set to new SlimMlsSettings() to enable E2E encryption
        MaxRetries = 5,
        RetryInterval = TimeSpan.FromSeconds(5),
        Metadata = new Dictionary<string, string>()
    };

    // Create the group session on the given channel
    using var session = await app.CreateSessionAsync(channelName, config);

    Console.WriteLine($"Group session created on channel: {channelName}");
    ```

=== "React Native"

    Group sessions are not yet supported in the React Native bindings.

### Invite a Participant

The session creator acts as a moderator and can invite other applications to join:

=== "Python"

    ```python
    async def invite_participant(app, session, participant_name, conn_id):
        # Set the route to the participant first
        await app.set_route_async(participant_name, conn_id)

        # Invite the participant — this performs discovery + MLS key exchange
        handle = await session.invite_async(participant_name)
        await handle.wait_async()

        print(f"Invited {participant_name} to the group")
    ```

=== "Go"

    ```go
    func inviteParticipant(app *slim.App, session *slim.Session, name slim.Name, connID slim.ConnID) error {
        // Set the route to the participant first
        if err := app.SetRouteAsync(name, connID); err != nil {
            return err
        }

        // Invite the participant — this performs discovery + MLS key exchange
        if err := session.InviteAndWaitAsync(name); err != nil {
            return err
        }
        return nil
    }
    ```

=== "Java"

    ```java
    void inviteParticipant(App app, Session session, Name participantName, Long connId) {
        // Set the route to the participant first
        app.setRoute(participantName, connId);

        // Invite the participant — this performs discovery + MLS key exchange
        session.inviteAndWait(participantName);

        System.out.println("Invited " + participantName + " to the group");
    }
    ```

=== "Kotlin"

    ```kotlin
    suspend fun inviteParticipant(app: App, session: Session, participantName: Name, connId: ULong) {
        // Set the route to the participant first
        app.setRouteAsync(participantName, connId)

        // Invite the participant — this performs discovery + MLS key exchange
        val handle = session.inviteAsync(participantName)
        handle.waitAsync()

        println("Invited $participantName to the group")
    }
    ```

=== "Node.js"

    Group sessions are not yet supported in the Node.js bindings.

=== ".NET"

    ```csharp
    // Set the route to the participant first
    using var inviteName = SlimName.Parse("myorg/default/participant");
    app.SetRoute(inviteName, connId);

    // Invite the participant (synchronous)
    session.Invite(inviteName);

    Console.WriteLine($"Invited {inviteName} to the group");
    ```

=== "React Native"

    Group sessions are not yet supported in the React Native bindings.

### Broadcast to the Group

=== "Python"

    ```python
    # Broadcast to all participants
    await session.publish_async(b"hello everyone", None, None)

    # Receive messages from the group
    received = await session.get_message_async(
        timeout=datetime.timedelta(seconds=30)
    )
    print("Channel message:", received.payload.decode())
    ```

=== "Go"

    ```go
    // Broadcast to all participants
    if err := session.PublishAndWaitAsync([]byte("hello everyone"), nil, nil); err != nil {
        log.Fatal(err)
    }

    // Receive messages from the group
    timeout := 30 * time.Second
    msg, err := session.GetMessageAsync(&timeout)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Println("Channel message:", string(msg.Payload))
    ```

=== "Java"

    ```java
    // Broadcast to all participants
    session.publishAndWait("hello everyone".getBytes(), null, null);

    // Receive messages from the group
    ReceivedMessage msg = session.getMessage(Duration.ofSeconds(30));
    System.out.println("Channel message: " + new String(msg.payload()));
    ```

=== "Kotlin"

    ```kotlin
    // Broadcast to all participants
    session.publishAsync("hello everyone".toByteArray(), null, null)

    // Receive messages from the group
    val msg = session.getMessageAsync(Duration.ofSeconds(30))
    println("Channel message: " + String(msg.payload))
    ```

=== "Node.js"

    Group sessions are not yet supported in the Node.js bindings.

=== ".NET"

    ```csharp
    // Broadcast to all participants
    await session.PublishAsync("hello everyone");

    // Receive messages from the group
    var msg = await session.GetMessageAsync(TimeSpan.FromSeconds(30));
    Console.WriteLine($"Channel message: {msg.Text}");
    ```

=== "React Native"

    Group sessions are not yet supported in the React Native bindings.

## Enabling End-to-End Encryption

Set `enable_mls=True` (Python) or provide an `MlsSettings` object (Go, Java, Kotlin) in the `SessionConfig` to enable MLS-based end-to-end encryption. The session layer handles all key establishment automatically — your application code stays the same.

=== "Python"

    ```python
    session_config = slim_bindings.SessionConfig(
        session_type=slim_bindings.SessionType.POINT_TO_POINT,
        enable_mls=True,  # Enable MLS encryption
        max_retries=5,
        interval=datetime.timedelta(seconds=5),
    )
    ```

=== "Go"

    ```go
    config := slim.SessionConfig{
        SessionType: slim.SessionTypePointToPoint,
        MlsSettings: &slim.MlsSettings{
            HeaderIntegrityValidationPercent: 100,
        },
    }
    ```

=== "Java"

    ```java
    SessionConfig sessionConfig = new SessionConfig(
        SessionType.POINT_TO_POINT,
        5,                       // maxRetries
        Duration.ofSeconds(5),   // interval
        Map.of(),                // metadata
        new MlsSettings(100)     // Enable MLS encryption
    );
    ```

=== "Kotlin"

    ```kotlin
    val sessionConfig = SessionConfig(
        sessionType = SessionType.POINT_TO_POINT,
        maxRetries = 5u,
        interval = Duration.ofSeconds(5),
        metadata = emptyMap(),
        mlsSettings = MlsSettings(100u) // Enable MLS encryption
    )
    ```

=== "Node.js"

    ```typescript
    const sessionConfig = {
        sessionType: "pointToPoint" as const,
        enableMls: true, // Enable MLS encryption
        maxRetries: 5,
        interval: 5000,
        metadata: new Map()
    };
    ```

=== ".NET"

    ```csharp
    var config = new SlimSessionConfig
    {
        SessionType = SlimSessionType.PointToPoint,
        MlsSettings = new SlimMlsSettings()  // Enable MLS encryption
    };
    ```

=== "React Native"

    ```tsx
    const sessionConfig = {
        sessionType: slimBindings.SessionType.PointToPoint,
        enableMls: true,  // Enable MLS encryption
        metadata: new Map()
    };
    ```

## Runnable Examples

The [slim-bindings repository](https://github.com/agntcy/slim-bindings) contains complete, runnable examples:

- [Python point-to-point example](https://github.com/agntcy/slim-bindings/blob/main/python/examples/point_to_point.py)
- [Python group example](https://github.com/agntcy/slim-bindings/blob/main/python/examples/group.py)
- [Go examples](https://github.com/agntcy/slim-bindings-go/tree/main/examples)
- [Java examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples)
- [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples)
- [Node.js examples](https://github.com/agntcy/slim-bindings/tree/main/node/examples)

## Next Steps

- [Sessions](../../architecture/sessions/index.md) — Deep dive into session types, sequence diagrams, and the full API
- [Groups](../../architecture/sessions/group.md) — Group creation and membership management via the SLIM Controller
- [Group Communication Tutorial](./tutorial-group.md) — End-to-end tutorial using identity and authentication
