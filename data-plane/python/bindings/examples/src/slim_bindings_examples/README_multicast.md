# Multicast Example with SLIM Python Bindings


This example demonstrates how to use the SLIM Python bindings to create and
manage a multicast session between distributed application instances. Multicast
enables one-to-many and many-to-many communication, where messages sent to a
channel are delivered to all participants. The test application allows you to
create a multicast group, invite participants, and exchange messages. This is
useful for chat-like applications, collaborative applications, or any scenario
where multiple distributed services need to communicate as a group.

## Features
- Create a multicast session as a moderator
- Invite multiple participants to the group
- Receive messages from the multicast channel
- Optionally enable Messaging Layer Security (MLS) for secure group messaging

## How It Works

### 1. Create the local application

The script initializes a local SLIM application instance using several configuration options:

```python
local_app = await create_local_app(
    local,
    slim,
    enable_opentelemetry=enable_opentelemetry,  # (bool, default: False)
    shared_secret=shared_secret,                # (str | None, default: None)
    jwt=jwt,                                    # (str | None, default: None)
    bundle=bundle,                              # (str | None, default: None)
    audience=audience,                          # (list[str] | None, default: None)
)
```



`create_local_app` is a helper function defined in `common.py` that creates and
configures a new local SLIM application instance. The main parameters are:



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
If neither `jwt` nor `bundle` is provided, `shared_secret` must be set. This
setting is discouraged in a production environment and should be used only for
testing purposes.

The part of the helper function that actually creates the local application and
connects it to the remote SLIM node is:
```python
    local_app = await slim_bindings.Slim.new(local_name, provider, verifier)

    format_message_print(f"{local_app.get_id()}", "Created app")

    # Connect to slim server
    _ = await local_app.connect(slim)
    format_message_print(f"{local_app.get_id()}", f"Connected to {slim['endpoint']}")
```


### 2. Create the moderator and invite participants


If the application is started as a moderator (i.e., both `remote` and `invites`
arguments are provided), the code creates a new multicast session as a
moderator and invites participants to join the group.

```python
session_info = await local_app.create_session(
    slim_bindings.PySessionConfiguration.Multicast(

        topic=broadcast_topic,           # (str or tuple) The multicast topic/channel name.
        moderator=True,                  # (bool) If True, this app can invite/remove participants.
        max_retries=5,                   # (int) Number of retransmissions for lost messages.
        timeout=datetime.timedelta(seconds=5), # (timedelta) Time between each retransmission atempt.
        mls_enabled=enable_mls,          # (bool) Enable Messaging Layer Security (MLS) for secure group messaging.
    )
)
```



- `topic`: The multicast channel or topic. This is a PyName that indicates the
    SLIM name of the channel.
- `moderator`: If `True`, this app can invite or remove participants from the
    session.
- `max_retries`: Number of retransmissions for lost messages. In this example,
    at most 5 retransmissions are allowed.
- `timeout`: Time between each retransmission attempt (set to 5 seconds here).
- `mls_enabled`: Enables secure group messaging using Messaging Layer Security
    (MLS). Set to `True` to enable encryption and authentication for all group
    messages.

Once the session is created, the application can invite new participants to the
channel using the `invite` function. This is done inside this loop:
```python
# Invite all participants
for p in invites:
    to_add = split_id(p)
    await local_app.set_route(to_add)
    await local_app.invite(session_info, to_add)
    print(f"{local} -> add {to_add} to the group")
```



Note: The `invite` function returns immediately, but it may require some time
to actually add the new participant to the channel. This is because several
message exchanges need to be performed in the background (e.g., for secure
group setup if MLS is enabled). You can check all the details of the invite
process in [SESSION.md](../../../SESSION.md).

TODO: the recv session and recv msg will change soon, continue this after the refactor.


## Usage


The recommended way to run the multicast example is via the Taskfile commands.
If you want to see all the command flags, check the
[Taskfile.yaml](../../Taskfile.yaml) file.

### 1. Start the SLIM server


Before running the multicast example, you need a running SLIM server. You can
start a local SLIM server using the Taskfile:

```bash
task python:example:server
```


This will start the SLIM server on `127.0.0.1:46357` by default.

### 2. Run the participants


Open two terminals and run the following commands to start the multicast
clients:

```bash
task python:example:multicast:client-1
```

```bash
task python:example:multicast:client-2
```


Each client will wait to be invited to the multicast channel and then receive
messages.

### 3. Run the multicast moderator


In a separate terminal, run:

```bash
task python:example:multicast:moderator
```


This will execute the moderator example, which creates a multicast channel and
invites participants.