# Tutorial: Creating a Session

This tutorial shows how to create point-to-point and group sessions to exchange messages between applications. Sessions provide reliable, optionally encrypted communication on top of the SLIM data plane.

## Prerequisites

- Completed [Creating an App](./tutorial-app.md)
- Two running SLIM applications or two instances of the same service

For conceptual background on session types, see [Sessions](../../architecture/session.md).

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
        "context"
        "fmt"
        "time"
        slim "github.com/agntcy/slim-bindings-go"
    )

    func runClient(ctx context.Context, app *slim.SlimApp, remoteName slim.Name) (*slim.Session, error) {
        // Configure the session
        sessionConfig := slim.SessionConfig{
            SessionType: slim.SessionTypePointToPoint,
            EnableMls:   false, // Set true to enable end-to-end encryption
            MaxRetries:  5,
            Interval:    5 * time.Second,
        }

        // Create the session — discovery happens automatically
        sessionCtx, err := app.CreateSession(ctx, sessionConfig, remoteName)
        if err != nil {
            return nil, err
        }

        // Wait until the session is fully established
        if err := sessionCtx.Completion.Wait(ctx); err != nil {
            return nil, err
        }

        fmt.Println("Point-to-point session established")
        return sessionCtx.Session, nil
    }
    ```

=== "Kotlin"

    Refer to the [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples) in the slim-bindings repository.

=== ".NET"

    Refer to the [.NET examples](https://github.com/agntcy/slim-bindings/tree/main/dotnet/examples) in the slim-bindings repository.

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
    // Send a message
    if err := session.Publish(ctx, []byte("hello"), nil, nil); err != nil {
        panic(err)
    }

    // Receive a reply
    msg, err := session.GetMessage(ctx, 30*time.Second)
    if err != nil {
        panic(err)
    }
    fmt.Println("Received:", string(msg.Payload))

    // Echo the message back
    if err := session.Publish(ctx, msg.Payload, nil, nil); err != nil {
        panic(err)
    }
    ```

=== "Kotlin"

    Refer to the [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples).

=== ".NET"

    Refer to the [.NET examples](https://github.com/agntcy/slim-bindings/tree/main/dotnet/examples).

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
    func createGroupSession(ctx context.Context, app *slim.SlimApp, channelName slim.Name) (*slim.Session, error) {
        sessionConfig := slim.SessionConfig{
            SessionType: slim.SessionTypeGroup,
            EnableMls:   false,
            MaxRetries:  5,
            Interval:    5 * time.Second,
        }

        // Create the session on the given channel
        sessionCtx, err := app.CreateSession(ctx, sessionConfig, channelName)
        if err != nil {
            return nil, err
        }

        // Wait until the session is ready
        if err := sessionCtx.Completion.Wait(ctx); err != nil {
            return nil, err
        }

        fmt.Println("Group session created")
        return sessionCtx.Session, nil
    }
    ```

=== "Kotlin"

    Refer to the [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples).

=== ".NET"

    Refer to the [.NET examples](https://github.com/agntcy/slim-bindings/tree/main/dotnet/examples).

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
    func inviteParticipant(ctx context.Context, app *slim.SlimApp, session *slim.Session, name slim.Name, connID slim.ConnID) error {
        // Set the route to the participant first
        if err := app.SetRoute(ctx, name, connID); err != nil {
            return err
        }

        // Invite the participant
        handle, err := session.Invite(ctx, name)
        if err != nil {
            return err
        }
        return handle.Wait(ctx)
    }
    ```

=== "Kotlin"

    Refer to the [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples).

=== ".NET"

    Refer to the [.NET examples](https://github.com/agntcy/slim-bindings/tree/main/dotnet/examples).

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
    if err := session.Publish(ctx, []byte("hello everyone"), nil, nil); err != nil {
        panic(err)
    }

    // Receive messages from the group
    msg, err := session.GetMessage(ctx, 30*time.Second)
    if err != nil {
        panic(err)
    }
    fmt.Println("Channel message:", string(msg.Payload))
    ```

=== "Kotlin"

    Refer to the [Kotlin examples](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples).

=== ".NET"

    Refer to the [.NET examples](https://github.com/agntcy/slim-bindings/tree/main/dotnet/examples).

## Enabling End-to-End Encryption

Set `enable_mls=True` (Python) or `EnableMls: true` (Go) in the `SessionConfig` to enable MLS-based end-to-end encryption. The session layer handles all key establishment automatically — your application code stays the same.

```python
session_config = slim_bindings.SessionConfig(
    session_type=slim_bindings.SessionType.POINT_TO_POINT,
    enable_mls=True,  # Enable MLS encryption
    max_retries=5,
    interval=datetime.timedelta(seconds=5),
)
```

## Runnable Examples

The [slim-bindings repository](https://github.com/agntcy/slim-bindings) contains complete, runnable examples:

- [Python point-to-point example](https://github.com/agntcy/slim-bindings/blob/main/python/examples/point_to_point.py)
- [Python group example](https://github.com/agntcy/slim-bindings/blob/main/python/examples/group.py)
- [Go examples](https://github.com/agntcy/slim-bindings-go/tree/main/examples)

## Next Steps

- [Sessions](../../architecture/session.md) — Deep dive into session types, sequence diagrams, and the full API
- [Groups](../../architecture/group.md) — Group creation and membership management via the SLIM Controller
- [Group Communication Tutorial](./tutorial-group.md) — End-to-end tutorial using identity and authentication
