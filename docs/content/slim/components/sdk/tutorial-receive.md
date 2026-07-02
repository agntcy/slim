# Tutorial: Receiving a Session

This tutorial shows how to receive an incoming session, read messages from it, and send replies. This is the counterpart to [Creating a Session](./tutorial-session.md), which covers the initiating side.

## Prerequisites

- Completed [Creating an App](./tutorial-app.md) — you need the `app` and `conn_id` objects

## Listen for an Incoming Session

The receiving side does not call `create_session`. Instead it calls `listen_for_session_async`, which blocks until the remote peer initiates a session (either a point-to-point connection or a group invitation).

=== "Python"

    ```python
    import asyncio
    import slim_bindings

    # Wait indefinitely for an incoming session
    session = await local_app.listen_for_session_async(None)

    print(f"Session received from: {session.source()}")
    ```

=== "Go"

    ```go
    import (
        "fmt"
        "log"

        slim "github.com/agntcy/slim-bindings-go"
    )

    // Wait indefinitely for an incoming session (nil = no timeout)
    session, err := app.ListenForSessionAsync(nil)
    if err != nil {
        log.Fatal(err)
    }

    fmt.Println("Session received")
    ```

=== "Java"

    ```java
    // Wait indefinitely for an incoming session (null = no timeout)
    Session session = app.listenForSession(null);

    System.out.println("Session received");
    ```

=== "Kotlin"

    ```kotlin
    // Wait indefinitely for an incoming session (null = no timeout)
    val session = app.listenForSessionAsync(null)

    println("Session received")
    ```

=== "Node.js"

    ```typescript
    // Wait up to 60 seconds for an incoming session (pass null to wait forever)
    const session = await app.listenForSessionAsync(60000);

    console.log("Session received");
    ```

=== ".NET"

    ```csharp
    // Wait up to 60 seconds for an incoming session
    using var session = await app.ListenForSessionAsync(TimeSpan.FromSeconds(60));

    Console.WriteLine($"Session received: {session.Destination}");
    ```

=== "React Native"

    ```tsx
    // Wait up to 60 seconds for an incoming session
    const session = await app.listenForSessionAsync(60000);

    console.log("Session received");
    ```

## Receive Messages

Once the session is established, call `get_message_async` in a loop to receive messages. The call blocks until a message arrives or the timeout expires.

=== "Python"

    ```python
    import datetime

    while True:
        try:
            msg = await session.get_message_async(
                timeout=datetime.timedelta(seconds=30)
            )
            print("Received:", msg.payload.decode())
        except Exception as e:
            if "session closed" in str(e).lower():
                break
            continue
    ```

=== "Go"

    ```go
    import (
        "fmt"
        "time"
    )

    for {
        timeout := 30 * time.Second
        msg, err := session.GetMessageAsync(&timeout)
        if err != nil {
            break
        }
        fmt.Println("Received:", string(msg.Payload))
    }
    ```

=== "Java"

    ```java
    import java.time.Duration;

    while (true) {
        try {
            ReceivedMessage msg = session.getMessage(Duration.ofSeconds(30));
            System.out.println("Received: " + new String(msg.payload()));
        } catch (Exception e) {
            break;
        }
    }
    ```

=== "Kotlin"

    ```kotlin
    import java.time.Duration

    while (true) {
        try {
            val msg = session.getMessageAsync(Duration.ofSeconds(30))
            println("Received: " + String(msg.payload))
        } catch (e: Exception) {
            break
        }
    }
    ```

=== "Node.js"

    ```typescript
    while (true) {
        try {
            const msg = await session.getMessageAsync(30000);
            console.log("Received:", Buffer.from(msg.payload).toString());
        } catch (e) {
            break;  // Session closed or timeout
        }
    }
    ```

=== ".NET"

    ```csharp
    while (true)
    {
        try
        {
            var msg = await session.GetMessageAsync(TimeSpan.FromSeconds(30));
            Console.WriteLine($"Received: {msg.Text}");
        }
        catch (Exception ex) when (ex.Message.Contains("timeout"))
        {
            continue;
        }
        catch
        {
            break;  // Session closed
        }
    }
    ```

=== "React Native"

    ```tsx
    while (true) {
        try {
            const msg = await session.getMessageAsync(30000);
            const text = String.fromCharCode(...new Uint8Array(msg.payload));
            console.log("Received:", text);
        } catch (e) {
            break;  // Session closed or timeout
        }
    }
    ```

## Reply to Messages

There are two ways to reply:

- **Broadcast** (`publish_async` / `PublishAndWaitAsync`) — sends to all current session participants. For point-to-point sessions this is just the remote peer; for group sessions every member receives it.
- **Direct reply** (`publish_to_async` / `PublishToAndWaitAsync` / `ReplyAsync`) — uses the context from the received message to send back only to the original sender. Other group participants do not see the reply.

=== "Python"

    ```python
    # Broadcast to all participants
    await session.publish_async(b"hello everyone", None, None)

    # Reply only to the sender
    await session.publish_to_async(
        msg.context,
        b"hello back",
        None,   # payload_type
        {},     # metadata
    )
    ```

=== "Go"

    ```go
    // Broadcast to all participants
    if err := session.PublishAndWaitAsync([]byte("hello everyone"), nil, nil); err != nil {
        log.Fatal(err)
    }

    // Reply only to the sender
    if err := session.PublishToAndWaitAsync(msg.Context, []byte("hello back"), nil, nil); err != nil {
        log.Fatal(err)
    }
    ```

=== "Java"

    ```java
    // Broadcast to all participants
    session.publishAndWait("hello everyone".getBytes(), null, null);

    // Reply only to the sender
    session.publishToAndWait(msg.context(), "hello back".getBytes(), null, null);
    ```

=== "Kotlin"

    ```kotlin
    // Broadcast to all participants
    session.publishAsync("hello everyone".toByteArray(), null, null)

    // Reply only to the sender
    session.publishToAsync(msg.context, "hello back".toByteArray(), null, null)
    ```

=== "Node.js"

    ```typescript
    // Broadcast to all participants
    await session.publishAndWaitAsync(Buffer.from("hello everyone"), undefined, undefined);

    // Reply only to the sender
    await session.publishToAndWaitAsync(
        msg.context,
        Buffer.from("hello back"),
        undefined,
        undefined
    );
    ```

=== ".NET"

    ```csharp
    // Broadcast to all participants
    await session.PublishAsync("hello everyone");

    // Reply only to the sender
    await session.ReplyAsync(msg, "hello back");
    ```

=== "React Native"

    ```tsx
    // Broadcast to all participants
    const broadcast = new Uint8Array("hello everyone".split('').map(c => c.charCodeAt(0)));
    await session.publishAndWaitAsync(broadcast, undefined, undefined);

    // Reply only to the sender
    const reply = new Uint8Array("hello back".split('').map(c => c.charCodeAt(0)));
    await session.publishToAndWaitAsync(msg.context, reply, undefined, undefined);
    ```

## Next Steps

- [Sessions](../../architecture/sessions/index.md) — Deep dive into session types, sequence diagrams, and the full API
- [Groups](../../architecture/sessions/group.md) — Group creation and membership management via the SLIM Controller
