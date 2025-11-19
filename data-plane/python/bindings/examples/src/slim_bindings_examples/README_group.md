# Group Example with SLIM Python Bindings

This example shows how to use the SLIM Python bindings to create and manage a
group messaging session between distributed application instances.
Group sessions enable many-to-many communication: any message
published to the channel is delivered to every current participant. This is
useful for chat, collaborative tools, live telemetry, or coordinated
control signals.

## Features
- Create a group session (the creator implicitly acts as moderator)
- Invite multiple participants to join dynamically
- Receive messages (with sender context) from the group channel
- Optionally enable Messaging Layer Security (MLS) for end‑to‑end secure group messaging

## How It Works

### 1. Create the local application

The script first initializes a local SLIM application instance using the
following configuration options:

```python
    # Create & connect the local Slim instance (auth derived from args).
    local_app = await create_local_app(
        local,
        slim,
        enable_opentelemetry=enable_opentelemetry,
        shared_secret=shared_secret,
        jwt=jwt,
        spire_trust_bundle=spire_trust_bundle,
        audience=audience,
        spire_socket_path=spire_socket_path,
        spire_target_spiffe_id=spire_target_spiffe_id,
        spire_jwt_audience=spire_jwt_audience,
    )
```

`create_local_app` (in `common.py`) creates and configures a new SLIM
application instance. Main parameters:


- `local` (str): The SLIM name of the local application in the form
    `org/ns/service` (required).
- `slim` (dict): Configuration to connect to the remote SLIM node. Example:
    ```python
    {
            "endpoint": "http://127.0.0.1:46357",
            "tls": {"insecure": True},
    }
    ```
    (required)
- `enable_opentelemetry` (bool, default: `False`): Enable OpenTelemetry
    tracing. If `True`, traces are sent to `http://localhost:4317` by default.
- `shared_secret` (str | None, default: `None`): Shared secret for identity and
    authentication. Required if JWT, bundle and audience are not provided.
- `jwt` (str | None, default: `None`): JWT token for identity. Used with
    `spire_trust_bundle` and `audience` for JWT-based authentication.
- `spire_trust_bundle` (str | None, default: `None`): JWT trust bundle (list
    of JWKs, one for each trust domain). It is expected in JSON format such as
    ```json
    {
        "trust-domain-1.org": "base-64-encoded-jwks",
        "trust-domain-2.org": "base-64-encoded-jwks",
        ...
    }
    ```
- `audience` (list[str] | None, default: `None`): List of allowed audiences for
    JWT authentication.
- `spire_socket_path` (str | None, default: `None`): Path to SPIRE agent socket for workload API access.
- `spire_target_spiffe_id` (str | None, default: `None`): Target SPIFFE ID for mTLS authentication with SPIRE.
- `spire_jwt_audience` (list[str] | None, default: `None`): Audience list for SPIRE JWT-SVID validation.

If `jwt`, `spire-trust-bundle` and `audience` are not provided, `shared_secret` must be set (only
recommended for local testing / examples, not production).

The code that creates the local application and connects it to the remote
SLIM node is:
```python

    # Convert local identifier to a strongly typed Name.
    local_name = split_id(local)

    # Instantiate Slim (async constructor prepares underlying Service).
    local_app = slim_bindings.Slim(local_name, provider, verifier)

    # Provide feedback to user (instance numeric id).
    format_message_print(f"{local_app.id_str}", "Created app")

    # Establish outbound connection using provided parameters.
    _ = await local_app.connect(slim)

    # Confirm endpoint connectivity.
    format_message_print(f"{local_app.id_str}", f"Connected to {slim['endpoint']}")
```

### 2. Create the session and invite participants

If the application is started with both a `--remote` (group channel name)
and at least one `--invites` flag, it becomes the creator of a new group
session and can invite participants.

```python
    # We are the moderator; create the group session now.
    format_message_print(
        f"Creating new group session (moderator)... {split_id(local)}"
    )
    config = slim_bindings.SessionConfiguration.Group(
        max_retries=5,  # Max per-message resend attempts upon missing ack before reporting a delivery failure.
        timeout=datetime.timedelta(
            seconds=5
        ),  # Ack / delivery wait window; after this duration a retry is triggered (until max_retries).
        mls_enabled=enable_mls,  # Enable Messaging Layer Security for end-to-end encrypted & authenticated group communication.
    )

    created_session, handle = await local_app.create_session(
        chat_channel,  # Logical group channel (Name) all participants join; acts as group/topic identifier.
        config,  # session configuration
    )

    await handle

    # Invite each provided participant. Route is set before inviting to ensure
    # outbound control messages can reach them. For more info see
    # https://github.com/agntcy/slim/blob/main/data-plane/python/bindings/SESSION.md#invite-a-new-participant
    for invite in invites:
        invite_name = split_id(invite)
        await local_app.set_route(invite_name)
        handle = await created_session.invite(invite_name)
        await handle
        print(f"{local} -> add {invite_name} to the group")
```

The `local_app.create_session(...)` creates the session and returns a handler `handle`. 
The `await handle` guarantees that the session is fully established when it returns.  
The `session.invite(...)` works in the same way. The `await handle` returns when the
remote participant is correctly added to the group. During the invite the background protocol
exchanges (and MLS key schedule if enabled) may take a short time before the
participant fully joins. See [SESSION.md](../../../SESSION.md) for deeper
protocol details.

### 3. Receiving messages (all participants)

Non-moderator participants (clients) start without a session and wait to be
invited:

```python
    print_formatted_text("Waiting for session...", style=custom_style)
    session = await local_app.listen_for_session()
```

Once a session is available (from creation or invite), messages are received in a loop:

```python
while True:
    try:
        # Await next inbound message from the group session.
        # The returned parameters are a message context and the raw payload bytes.
        # Check session.py for details on MessageContext contents.
        ctx, payload = await session.get_message()
        print_formatted_text(
            f"{ctx.source_name} > {payload.decode()}",
            style=custom_style,
        )
```
The message is received with `session.get_message()`. `ctx` is a `MessageContext`
with information about the received message. `ctx.source_name` is the `Name` of
the sender, and `payload` is a `bytes` object carrying the published message.

### 4. Publishing messages

Every participant also runs an interactive input loop that allows typing
messages which are immediately published to the group:

```python
    while True:
        # Run blocking input() in a worker thread so we do not block the event loop.
        user_input = await prompt_session.prompt_async(
            f"{shared_session_container[0].src} > "
        )

        if user_input.lower() in ("exit", "quit"):
            # Also terminate the receive loop.
            handle = await local_app.delete_session(shared_session_container[0])
            await handle
            break

        # Send message to the channel_name specified when creating the session.
        # As the session is group, all participants will receive it.
        await shared_session_container[0].publish(user_input.encode())
```

The message is published using `shared_session_container[0].publish(user_input.encode())`
and delivered to all the participants connected to the group.

In case the user inputs the string "exit" or "quit" the `local_app.delete_session(shared_session_container[0])`
is executed and this deletes the session. If `delete_session` is called on a moderator 
the session is deleted on all the participants in the group. The `await handle` guarantees
that all the participants have been notified and correctly removed from the group.

## Usage

Use the Taskfile commands for reproducible local runs. See
[Taskfile.yaml](../../Taskfile.yaml) for all options.

### 1. Start the SLIM server

Start a local SLIM server:

```bash
task python:example:server
```


By default this listens on `127.0.0.1:46357`.

### 2. Start participants (receivers)

Open two terminals and run:

```bash
task python:example:group:client-1
```

```bash
task python:example:group:client-2
```


Each client waits to be invited.

### 3. Start the moderator

In a third terminal run:

```bash
task python:example:group:moderator
```


This creates the channel (`agntcy/ns/chat`) and invites the two clients.
After the invite, you can start to send messages by typing on any one of the
terminals. The messages will arrive to all the participants in the group.
