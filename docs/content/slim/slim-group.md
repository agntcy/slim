# SLIM Group Creation and Management

One of the key features of [SLIM](../index.md) is its support for secure group communication.
In SLIM, a group consists of multiple clients that communicate through a shared
channel. Each channel is identified by a unique name, as described in the [SLIM
Messaging Layer](slim-data-plane.md). When MLS is enabled, group
communication benefits from end-to-end encryption.

This guide provides all the information you need to create and manage groups within a
SLIM network. A full tutorial with examples is available in
[Group Communication Tutorial](./slim-group-tutorial.md).

## Creating Groups with SLIM Bindings

This section shows how to use the SLIM bindings to create a group.
This requires a [group session](./slim-session.md#group-session). A group
session is a channel shared among multiple participants and used to
send messages to everyone. When a new participant wants to join the channel,
they must be invited by the channel creator.

The channel creator can be part of the application and can either
actively participate in the communication process (possibly implementing some
of the application logic) or serve solely as a channel moderator.

Group creation is available in both Python and Go bindings. This section provides
the basic steps to follow with Python code snippets for setting up a group session.

For Go examples, see the [group example](https://github.com/agntcy/slim-bindings/tree/main/go/examples/group/main.go).

The full Python code is available in [group.py](https://github.com/agntcy/slim-bindings/tree/main/python/examples/group.py).

### Create the Channel

The channel can be created with a group session,
which initializes the corresponding state in the SLIM session layer.
In a group session, communication between participants can be encrypted
end-to-end, enabling MLS.

```python
# Create group session configuration
session_config = slim_bindings.SessionConfig(
    session_type=slim_bindings.SessionType.GROUP,
    enable_mls=enable_mls,  # Enable Messaging Layer Security for end-to-end encrypted & authenticated group communication.
    max_retries=5,  # Max per-message resend attempts upon missing ack before reporting a delivery failure.
    interval=datetime.timedelta(seconds=5),  # Ack / delivery wait window; after this duration a retry is triggered (until max_retries).
    metadata={},
)

# Create session - returns a tuple (SessionContext, CompletionHandle)
session = local_app.create_session(session_config, chat_channel)
# Wait for session to be established
await session.completion.wait_async()
created_session = session.session
```

### Invite Participants to the Channel

Once the group session is created, new participants can be invited
to join. Not all participants need to be added at the beginning;
you can add them later, even after communication has started.

```python
for invite in invites:
    invite_name = split_id(invite)
    await local_app.set_route_async(invite_name, conn_id)
    handle = await created_session.invite_async(invite_name) # invite participant
    await handle.wait_async()   # await for the invite to be finished
    print(f"{local} -> add {invite_name} to the group")
```

### Listen for New Sessions and Messages

Participants that need to join the group start without a session and wait to be
invited. To wait for an invitation, the application calls `listen_for_session`.
When an invite message is received, a new session is created at the SLIM session layer,
and `listen_for_session` returns the metadata for the newly created session.

```python
print_formatted_text("Waiting for session...", style=custom_style)
session = await local_app.listen_for_session_async(None)  # timeout: datetime.timedelta | None = wait indefinitely
```

When a new session is available, the participant can start listening for messages:

```python
while True:
    try:
        # Await next inbound message from the group session.
        # Returns a ReceivedMessage object with context and payload.
        received_msg = await session.get_message_async(
            timeout=datetime.timedelta(seconds=30)
        )
        ctx = received_msg.context
        payload = received_msg.payload

        # Display sender name and message
        sender = ctx.source_name if hasattr(ctx, "source_name") else source_name
        print_formatted_text(
            f"{sender} > {payload.decode()}",
            style=custom_style,
        )
```

### Send Messages on a Channel

Each participant can also send messages at any time to the new session, and each message will be delivered to all participants connected to the same channel.

```python
# Send message to the channel_name specified when creating the session.
# As the session is group, all participants will receive it.
await shared_session_container[0].publish_async(
    user_input.encode(),  # payload: bytes
    None,  # payload_type: str | None
    None   # metadata: dict[str, str] | None
)
```

## Creating Groups with the Channel Manager

Another way to create a group in a SLIM network is to use the
[Channel Manager](./slim-channel-manager.md). The channel manager is a separate
service that creates group sessions and invites participants on your behalf, so
applications only need to wait for session invitations.

For a complete walkthrough, see the
[Group Communication Tutorial](./slim-group-tutorial.md). This section lists the
`slimctl` commands equivalent to the bindings workflow above.

### Prerequisites

1. A running SLIM node (data plane on port 46357)
2. A running [channel manager](./slim-channel-manager.md) instance (API on port 10356)
3. Application instances connected to the SLIM node

### Create the Channel

Create a named channel. Choose a channel name in `org/namespace/channel` format:

```bash
slimctl channel-manager create-channel agntcy/ns/my-group
```

Expected output:

```text
Channel agntcy/ns/my-group created successfully
```

### Add Participants

Add participants by their application identity (`org/namespace/app`):

```bash
slimctl channel-manager add-participant agntcy/ns/my-group agntcy/ns/client-1
slimctl channel-manager add-participant agntcy/ns/my-group agntcy/ns/client-2
```

Expected output:

```text
Participant agntcy/ns/client-2 added to channel agntcy/ns/my-group
```

Participants receive a session invitation automatically. Message reception and
publishing work the same way as in the bindings example above.

### Remove a Participant

```bash
slimctl channel-manager delete-participant agntcy/ns/my-group agntcy/ns/client-2
```

### Delete the Channel

```bash
slimctl channel-manager delete-channel agntcy/ns/my-group
```

See the [Controller Reference](./slim-controller-reference.md#channel-manager-group-channel-management)
for all channel-manager commands.
