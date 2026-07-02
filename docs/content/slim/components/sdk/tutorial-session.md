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
        session_config = slim_bindings.SessionConfig(
            session_type=slim_bindings.SessionType.POINT_TO_POINT,
            enable_mls=True,
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
        config := slim.SessionConfig{
            SessionType: slim.SessionTypePointToPoint,
            MlsSettings: &slim.MlsSettings{
                HeaderIntegrityValidationPercent: 100,
            },
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
        SessionConfig sessionConfig = new SessionConfig(
            SessionType.POINT_TO_POINT,
            5,                       // maxRetries
            Duration.ofSeconds(5),   // interval
            Map.of(),                // metadata
            new MlsSettings(100)     // Enable E2E encryption
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
        val sessionConfig = SessionConfig(
            sessionType = SessionType.POINT_TO_POINT,
            maxRetries = 5u,
            interval = Duration.ofSeconds(5),
            metadata = emptyMap(),
            mlsSettings = MlsSettings(100u)
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
        const sessionConfig = {
            sessionType: "pointToPoint" as const,
            enableMls: true,
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

    var config = new SlimSessionConfig
    {
        SessionType = SlimSessionType.PointToPoint,
        MlsSettings = new SlimMlsSettings(),
        MaxRetries = 5,
        RetryInterval = TimeSpan.FromSeconds(5),
        Metadata = new Dictionary<string, string>()
    };

    // Create the session — discovery happens automatically
    // Blocks until the session is fully established
    using var session = await app.CreateSessionAsync(remoteName, config);

    Console.WriteLine("Point-to-point session established");
    ```

=== "React Native"

    ```tsx
    const sessionConfig = {
        sessionType: slimBindings.SessionType.PointToPoint,
        enableMls: true,
        maxRetries: 5,
        interval: 5000, // milliseconds
        metadata: new Map()
    };

    // Create the session — discovery happens automatically
    const session = await app.createSessionAndWaitAsync(sessionConfig, remoteName);

    console.log("Point-to-point session established");
    ```

## Group Session

A group session enables many-to-many communication on a named channel. Every message sent to the channel is delivered to all current participants.

### Create the Session

=== "Python"

    ```python
    async def create_group_session(app, channel_name):
        session_config = slim_bindings.SessionConfig(
            session_type=slim_bindings.SessionType.GROUP,
            enable_mls=True,
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
            MlsSettings: &slim.MlsSettings{
                HeaderIntegrityValidationPercent: 100,
            },
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
            new MlsSettings(100)     // Enable E2E encryption
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
            mlsSettings = MlsSettings(100u)
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

    ```typescript
    const sessionConfig = {
        sessionType: "group" as const,
        enableMls: true,
        maxRetries: 5,
        interval: 5000, // milliseconds
        metadata: new Map()
    };

    // Create the group session on the given channel
    const session = await app.createSessionAndWaitAsync(sessionConfig, channelName);

    console.log(`Group session created on channel: ${channelName}`);
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim;

    var config = new SlimSessionConfig
    {
        SessionType = SlimSessionType.Group,
        MlsSettings = new SlimMlsSettings(),
        MaxRetries = 5,
        RetryInterval = TimeSpan.FromSeconds(5),
        Metadata = new Dictionary<string, string>()
    };

    // Create the group session on the given channel
    using var session = await app.CreateSessionAsync(channelName, config);

    Console.WriteLine($"Group session created on channel: {channelName}");
    ```

=== "React Native"

    ```tsx
    const sessionConfig = {
        sessionType: slimBindings.SessionType.Group,
        enableMls: true,
        maxRetries: 5,
        interval: 5000, // milliseconds
        metadata: new Map()
    };

    // Create the group session on the given channel
    const session = await app.createSessionAndWaitAsync(sessionConfig, channelName);

    console.log(`Group session created on channel: ${channelName}`);
    ```

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

    ```typescript
    // Set the route to the participant first
    await app.setRoute(inviteName, Number(connId));

    // Invite the participant
    await session.inviteAndWaitAsync(inviteName);

    console.log(`Invited ${inviteName} to the group`);
    ```

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

    ```tsx
    // Set the route to the participant first
    await app.setRoute(inviteName, connId);

    // Invite the participant
    await session.inviteAndWaitAsync(inviteName);

    console.log(`Invited ${inviteName} to the group`);
    ```

## Send a Message

`publish_async` / `PublishAndWaitAsync` delivers the message to all current session participants. For point-to-point sessions this is just the single remote peer; for group sessions every member receives it.

=== "Python"

    ```python
    await session.publish_async(
        b"hello",   # payload: bytes
        None,       # payload_type: str | None
        None,       # metadata: dict | None
    )
    ```

=== "Go"

    ```go
    if err := session.PublishAndWaitAsync([]byte("hello"), nil, nil); err != nil {
        log.Fatal(err)
    }
    ```

=== "Java"

    ```java
    session.publishAndWait("hello".getBytes(), null, null);
    ```

=== "Kotlin"

    ```kotlin
    session.publishAsync("hello".toByteArray(), null, null)
    ```

=== "Node.js"

    ```typescript
    await session.publishAndWaitAsync(Buffer.from("hello"), undefined, undefined);
    ```

=== ".NET"

    ```csharp
    await session.PublishAsync("hello");
    ```

=== "React Native"

    ```tsx
    // Payload is Uint8Array in React Native
    const payload = new Uint8Array("hello".split('').map(c => c.charCodeAt(0)));
    await session.publishAndWaitAsync(payload, undefined, undefined);
    ```

## Listen for a Reply

After sending, call `get_message_async` to wait for an inbound message on the same session.

=== "Python"

    ```python
    import datetime

    received = await session.get_message_async(
        timeout=datetime.timedelta(seconds=30)
    )
    print("Received:", received.payload.decode())
    ```

=== "Go"

    ```go
    import "time"

    timeout := 30 * time.Second
    msg, err := session.GetMessageAsync(&timeout)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Println("Received:", string(msg.Payload))
    ```

=== "Java"

    ```java
    import java.time.Duration;

    ReceivedMessage msg = session.getMessage(Duration.ofSeconds(30));
    System.out.println("Received: " + new String(msg.payload()));
    ```

=== "Kotlin"

    ```kotlin
    import java.time.Duration

    val msg = session.getMessageAsync(Duration.ofSeconds(30))
    println("Received: " + String(msg.payload))
    ```

=== "Node.js"

    ```typescript
    const msg = await session.getMessageAsync(30000); // timeout in milliseconds
    console.log("Received:", Buffer.from(msg.payload).toString());
    ```

=== ".NET"

    ```csharp
    var msg = await session.GetMessageAsync(TimeSpan.FromSeconds(30));
    Console.WriteLine($"Received: {msg.Text}");
    ```

=== "React Native"

    ```tsx
    const msg = await session.getMessageAsync(30000);
    const text = String.fromCharCode(...new Uint8Array(msg.payload));
    console.log("Received:", text);
    ```

## Send to a Specific Participant

In a group session, `publish_to_async` / `PublishToAndWaitAsync` sends to a single participant using the context from a previously received message. Other group members do not see the message.

=== "Python"

    ```python
    # received is a ReceivedMessage obtained from session.get_message_async(...)
    await session.publish_to_async(
        received.context,
        b"private reply",
        None,   # payload_type
        {},     # metadata
    )
    ```

=== "Go"

    ```go
    // msg is obtained from session.GetMessageAsync(...)
    if err := session.PublishToAndWaitAsync(msg.Context, []byte("private reply"), nil, nil); err != nil {
        log.Fatal(err)
    }
    ```

=== "Java"

    ```java
    // msg is obtained from session.getMessage(...)
    session.publishToAndWait(msg.context(), "private reply".getBytes(), null, null);
    ```

=== "Kotlin"

    ```kotlin
    // msg is obtained from session.getMessageAsync(...)
    session.publishToAsync(msg.context, "private reply".toByteArray(), null, null)
    ```

=== "Node.js"

    ```typescript
    // msg is obtained from session.getMessageAsync(...)
    await session.publishToAndWaitAsync(
        msg.context,
        Buffer.from("private reply"),
        undefined,
        undefined
    );
    ```

=== ".NET"

    ```csharp
    // msg is obtained from session.GetMessageAsync(...)
    await session.ReplyAsync(msg, "private reply");
    ```

=== "React Native"

    ```tsx
    // msg is obtained from session.getMessageAsync(...)
    const payload = new Uint8Array("private reply".split('').map(c => c.charCodeAt(0)));
    await session.publishToAndWaitAsync(msg.context, payload, undefined, undefined);
    ```

## Advanced Session Config

### Reliability

By default, if `max_retries` and `interval` are omitted (or set to `null`/`nil`), the session layer sends messages fire-and-forget — no acknowledgement is requested and no retransmission occurs.

When `max_retries` and `interval` are set, the session layer requests an acknowledgement for each message. If no ack arrives within `interval`, the message is resent. This repeats up to `max_retries` times before the session reports a delivery failure to your application.

The code examples in this tutorial use `max_retries=5` and a 5-second interval. Set both to `null`/`nil`/`0` for unreliable (fire-and-forget) delivery.

### End-to-End Encryption

Set `enable_mls=True` (Python) or provide an `MlsSettings` object (Go, Java, Kotlin) in the `SessionConfig` to enable MLS-based end-to-end encryption. The session layer handles all key establishment automatically — your application code stays the same.

The examples in this tutorial already have MLS enabled. To disable it, set `enable_mls=False` / `MlsSettings = null` / `MlsSettings: nil`.

## Runnable Examples

The [slim-bindings repository](https://github.com/agntcy/slim-bindings) contains complete, runnable examples:

- [Python point-to-point example](https://github.com/agntcy/slim-bindings/blob/main/python/examples/point_to_point.py)
- [Python group example](https://github.com/agntcy/slim-bindings/blob/main/python/examples/group.py)
- [Go examples](https://github.com/agntcy/slim-bindings-go/tree/main/examples)
- [Java examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples)
- [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples)
- [Node.js examples](https://github.com/agntcy/slim-bindings/tree/main/node/examples)

## Next Steps

- [Receiving a Session](./tutorial-receive.md) — Listen for incoming sessions, receive messages, and reply
- [Sessions](../../architecture/sessions/index.md) — Deep dive into session types, sequence diagrams, and the full API
- [Groups](../../architecture/sessions/group.md) — Group creation and membership management via the SLIM Controller
