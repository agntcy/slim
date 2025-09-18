# Multicast Example with SLIM Python Bindings

This example demonstrates how to use the SLIM Python bindings to create and manage a multicast session between distributed application instances. The script allows you to create a multicast group, invite participants, and exchange messages in real time.

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


`create_local_app` is an helper function that is defined in common.py that creates a new local app.
The list of parameters to pass is the the following:

- `local` (str): Is the SLIM name of the local application in the form `org/ns/service` (required).
- `slim` (dict): Configuration to connec to the remote SLIM node. The example used 
        ```{
            "endpoint": "http://127.0.0.1:46357",
            "tls": {
                "insecure": True,
            },
        }``` as default (required).
- `enable_opentelemetry` (bool, default: `False`): Enable OpenTelemetry tracing. If `True`, tracing is initialized and sent to `http://localhost:4317` by default.
- `shared_secret` (str | None, default: `None`): Shared secret for identity and authentication. Required if JWT and bundle are not provided.
- `jwt` (str | None, default: `None`): JWT token for identity. Used with `bundle` and `audience` for JWT-based authentication.
- `bundle` (str | None, default: `None`): JWT trust bundle (CA certificates or JWKS).
- `audience` (list[str] | None, default: `None`): List of allowed audiences for JWT authentication.

Using this helper function if neither `jwt` nor `bundle` is provided, `shared_secret` must be set. This setting is discuraged in a production
environemnt and should be used only for testing parposes. The part of the helper function that actually create the local application and 
connects it to the remote SLIM node is
```python
    local_app = await slim_bindings.Slim.new(local_name, provider, verifier)

    format_message_print(f"{local_app.get_id()}", "Created app")

    # Connect to slim server
    _ = await local_app.connect(slim)
    format_message_print(f"{local_app.get_id()}", f"Connected to {slim['endpoint']}")
```

### 2. Crate the moderator and invite participants

If the application start as a moderator (`remote` and `invites` arguments are provided), the script creates a new
multicast session as a moderator and invites participants.

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

- `topic`: The multicast channel or topic. this is a PyName that indicate SLIM name of the channle.
- `moderator`: If `True`, this app can invite or remove participants from the session.
- `max_retries`: Number of retransmissions for lost messages. In the example we allow for at most 5 retransmissions
- `timeout`: Time between each retransmission atempt which is set to 5 seconds.
- `mls_enabled`: Enables secure group messaging using Messaging Layer Security (MLS). Set to `True` to enable encryption and authentication for all group messages.

Now that the session is created the application can invite new participant to the channel using the 
`invite` function. This is done inside this loop.
```python
# Invite all participants
for p in invites:
    to_add = split_id(p)
    await local_app.set_route(to_add)
    await local_app.invite(session_info, to_add)
    print(f"{local} -> add {to_add} to the group")
```

Notice that the invite function returns immediatly but it may require some time to actually add the new participant to the channel.
This happens because few message exchange need to be performed. You can check all the details of the invite function in [SESSION.md](../../../SESSION.md).

TODO: the recv session and recv msg will change soon, continue this after the refactor.



## Usage

The recommended way to run the multicast example is via the Taskfile command. If you want to see all the command flags, check the [Taskfile.yaml](../../Taskfile.yaml) file.

### 1. Start the SLIM server

Before running the multicast example, you need a running SLIM server. You can start a local SLIM server using the Taskfile:

```bash
task python:example:server
```

This will start the SLIM server on `127.0.0.1:46357` by default.

### 2. Run the multicast moderator 

```bash
task python:example:multicast:moderator
```

This will execute the moderator example, which creates a multicast channel and invites participants. You can customize the arguments by editing the Taskfile or passing `EXTRA_ARGS` as needed.

TODO: wait for the new taskfile to contiue
