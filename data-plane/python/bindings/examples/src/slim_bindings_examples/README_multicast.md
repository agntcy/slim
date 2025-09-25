# Multicast Example with SLIM Python Bindings

This example shows how to use the SLIM Python bindings to create and manage a
multicast (group) messaging session between distributed application instances.
Multicast enables one-to-many and many-to-many communication: any message
published to the channel is delivered to every current participant. This is
useful for chat, collaborative tools, live telemetry, or coordinated
control signals.

## Features
- Create a multicast session (the creator implicitly acts as moderator)
- Invite multiple participants to join dynamically
- Receive messages (with sender context) from the multicast channel
- Optionally enable Messaging Layer Security (MLS) for end‑to‑end secure group messaging
- Interactive publishing from the moderator terminal

## How It Works

### 1. Create the local application

The script first initializes a local SLIM application instance using several configuration options:

```python
local_app = await create_local_app(
    local,
    slim,
    enable_opentelemetry=enable_opentelemetry,  # (bool, default False)
    shared_secret=shared_secret,                # (str | None)
    jwt=jwt,                                    # (str | None)
    bundle=bundle,                              # (str | None)
    audience=audience,                          # (list[str] | None)
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
    authentication. Required if JWT and bundle are not provided.
- `jwt` (str | None, default: `None`): JWT token for identity. Used with
    `bundle` and `audience` for JWT-based authentication.
- `bundle` (str | None, default: `None`): JWT trust bundle (CA certificates or
    JWKS).
- `audience` (list[str] | None, default: `None`): List of allowed audiences for
    JWT authentication.
If neither `jwt` nor `bundle` is provided, `shared_secret` must be set (only
recommended for local testing / examples, not production).

The part that actually creates the local application and connects it to the
remote SLIM node is:
```python
local_app = await slim_bindings.Slim.new(local_name, provider, verifier)
format_message_print(f"{local_app.id}", "Created app")
_ = await local_app.connect(slim)
format_message_print(f"{local_app.id}", f"Connected to {slim['endpoint']}")
```
### 2. Create the session and invite participants

If the application is started with both a `--remote` (multicast channel name) and
at least one `--invites` flag, it becomes the creator of a new
multicast session and it can invite participants.

```python
chat_topic = split_id(remote)  # e.g. agntcy/ns/chat
session = await local_app.create_session(
    slim_bindings.PySessionConfiguration.Multicast(
        topic=chat_topic,
        max_retries=5,                             # max retransmissions for lost messages
        timeout=datetime.timedelta(seconds=5),     # interval between retries
        mls_enabled=enable_mls,                    # enable MLS secure group messaging
    )
)

await asyncio.sleep(1)  # small slack before inviting
for invite in invites:
    invite_name = split_id(invite)
    await local_app.set_route(invite_name)  # ensure routing info is set
    await session.invite(invite_name)
    print(f"{local} -> add {invite_name} to the group")
```

The `session.invite(...)` call returns quickly; background protocol exchanges
(and MLS key schedule if enabled) may take a short time before the participant
fully joins. See [SESSION.md](../../../SESSION.md) for deeper protocol details.

### 3. Receiving messages (all participants)

Non‑moderator participants (clients) start without a session and wait to be
invited:

```python
format_message_print(local, "-> Waiting for session...")
session = await local_app.listen_for_session()
```

Once a session is available (from creation or invite), messages are received in a loop:

```python
while True:
    ctx, payload = await session.get_message()
    format_message_print(
        local,
        f"-> Received message from {ctx.source_name}: {payload.decode()}",
    )
```

`ctx.source_name` is the `PyName` of the sender; `payload` is a `bytes` object
carrying the published message.

### 4. Publishing messages (moderator / any publisher)

The moderator example also provides an interactive input loop so you can type
messages which are immediately published to the multicast group:

```python
while True:
    user_input = input("message> ")
    if user_input.strip().lower() in ("exit", "quit"):
        break
    await session.publish(user_input.encode())
```

Any participant with the appropriate permissions can call `session.publish` in
similar fashion (the provided example only wires the loop for the creating
process for clarity).


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
task python:example:multicast:client-1
```

```bash
task python:example:multicast:client-2
```


Each client waits to be invited, then prints any received messages.

### 3. Start the moderator (creator + interactive publisher)

In a third terminal run:

```bash
task python:example:multicast:moderator
```


This creates the channel (`agntcy/ns/chat`), invites the two clients, and then
lets you type messages. Type `exit` or `quit` to stop the moderator. Clients
will continue printing messages until you terminate them (Ctrl+C).

### 4. (Optional) Enable MLS

Append `EXTRA_ARGS=--enable-mls` when running any of the Task targets to enable
secure group messaging. Example:

```bash
task python:example:multicast:moderator EXTRA_ARGS=--enable-mls
```

All participants must use the same MLS setting for the session.