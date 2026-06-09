# SLIM Group Communication Tutorial

This tutorial shows how to set up secure group communication using
SLIM. The group is created by defining a group session and inviting
participants. Messages are sent to a shared channel where every member can read
and write. All messages are end-to-end encrypted using the
[MLS protocol](https://datatracker.ietf.org/doc/html/rfc9420). This tutorial is
based on the
[group.py](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/bindings/python/examples/group.py)
example in the SLIM repo.

## Key Features

- **Name-based Addressing**: In SLIM, all endpoints (channels and clients) have
  a name, and messages use a name-based addressing scheme for content routing.
- **Session Management**: Allows for the creation and management of sessions using
    both the SLIM Python Bindings and the SLIM Controller.
- **Broadcast Messaging**: Facilitates broadcast messaging to multiple
  subscribers.
- **End-to-End Encryption**: Ensures secure communication using the [MLS
  protocol](https://datatracker.ietf.org/doc/html/rfc9420).

## Configure Client Identity and Implement the SLIM App

Every participant in a group requires a unique identity for authentication and for use by the MLS protocol. This section explains how to set up identity and create a SLIM application instance.

### SLIM App

Every SLIM application requires both a unique identity and an authentication mechanism. The identity is used for end-to-end encryption via the MLS protocol, while authentication verifies the application to the SLIM network. In this tutorial, we use shared secret authentication for simplicity. For more advanced authentication methods (JWT, SPIRE), see the [SLIM documentation](./slim-authentication.md).

The example code provides a `create_local_app` helper function (from [common.py](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/bindings/python/examples/common.py)) that simplifies the app creation and connection process.

The `create_local_app` function handles three main tasks:

1. Initialize the Global Service: Sets up the SLIM runtime and global service instance
2. Create Authentication: Determines the authentication mode (SPIRE, JWT, or shared secret) based on configuration
3. Connect to SLIM Server: Establishes connection to the SLIM network

Here's how the function works:

```python
async def create_local_app(config: BaseConfig) -> tuple[slim_bindings.App, int]:
    """
    Build a Slim application instance using the global service.

    Resolution precedence for auth:
      1. If SPIRE options provided -> SPIRE dynamic identity flow.
      2. Else if jwt + bundle + audience provided -> JWT/JWKS flow.
      3. Else -> shared secret (must be provided).

    Args:
        config: BaseConfig instance containing all configuration.

    Returns:
        tuple[App, int]: Slim application instance and connection ID.
    """
    # Initialize tracing and global state
    service = setup_service()

    # Convert local identifier to a strongly typed Name.
    local_name = split_id(config.local)

    # Connect to SLIM server (returns connection ID)
    client_config = slim_bindings.new_insecure_client_config(config.slim)
    conn_id = await service.connect_async(client_config)

    # Determine authentication mode
    auth_mode = config.get_auth_mode()

    if auth_mode == AuthMode.SPIRE:
        print("Using SPIRE dynamic identity authentication.")
        provider_config, verifier_config = spire_identity(
            socket_path=config.spire_socket_path,
            target_spiffe_id=config.spire_target_spiffe_id,
            jwt_audiences=config.spire_jwt_audience,
        )
        local_app = service.create_app(local_name, provider_config, verifier_config)
    elif auth_mode == AuthMode.JWT:
        print("Using JWT + JWKS authentication.")
        # These should always be set if auth_mode is JWT
        if not config.jwt or not config.spire_trust_bundle:
            raise ValueError(
                "JWT and SPIRE trust bundle are required for JWT auth mode"
            )
        provider_config, verifier_config = jwt_identity(
            config.jwt,
            config.spire_trust_bundle,
            str(local_name),
            aud=config.audience,
        )
        local_app = service.create_app(local_name, provider_config, verifier_config)
    else:
        print("Using shared-secret authentication.")
        local_app = service.create_app_with_secret(local_name, config.shared_secret)

    # Provide feedback to user (instance numeric id).
    format_message_print(f"{local_app.id()}", "Created app")

    # Subscribe to the local name
    await local_app.subscribe_async(local_name, conn_id)

    return local_app, conn_id
```

### Key Authentication Options

The following key authentication options are available:

#### SPIRE (Recommended for production)
  
Uses the SPIRE Workload API for dynamic identity. Requires a running SPIRE agent. See [SLIM documentation](./slim-authentication.md) for setup details.

#### JWT/JWKS (Production)
  
Uses static JWT files with JWKS for verification. Suitable for environments with an existing JWT infrastructure.

#### Shared Secret (Development only)
  
Simple symmetric key authentication.

## Group Communication Using the Python Bindings

Now that you know how to set up a SLIM application, we can see how to create a group where multiple participants can exchange messages. We start by showing how to create a group session using the Python bindings.

In this setting, one participant acts as moderator: it creates the group session and invites participants by sending invitation control messages. A detailed description of group sessions and the invitation process is available in the [SLIM documentation](./slim-session.md).

### Creating the Group Session and Inviting Members

The creator of the group session invites other members to join the group. The
session will be identified by a unique session ID, and the group communication
will take place over a specific channel name. The session creator is responsible
for managing the session lifecycle, including creating, updating, and
terminating the session as needed.

As each participant is provided with an identity, setting up MLS for end-to-end
encryption is straightforward: the session is created with the
`mls_enabled` flag set to `True`, which will enable the MLS protocol for the
session. This ensures that all messages exchanged within the session are
end-to-end encrypted, providing confidentiality and integrity for the group
communication.

```python
# Create group session configuration
session_config = slim_bindings.SessionConfig(
    session_type=slim_bindings.SessionType.GROUP,
    enable_mls=enable_mls,  # Enable Messaging Layer Security for end-to-end encrypted & authenticated group communication.
    max_retries=5,  # Max per-message resend attempts upon missing ack before reporting a delivery failure.
    interval=datetime.timedelta(seconds=5),  # Ack / delivery wait window; after this duration a retry is triggered (until max_retries).
    metadata={},
)

# Create session - returns a SessionContext
session = local_app.create_session(session_config, chat_channel)
# Wait for session to be established
await session.completion.wait_async()
created_session = session.session

# Invite each provided participant
for invite in invites:
    invite_name = split_id(invite)
    await local_app.set_route_async(invite_name, conn_id)
    handle = await created_session.invite_async(invite_name)
    await handle.wait_async()
    print(f"{local} -> add {invite_name} to the group")
```

This code comes from the
[group.py](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/bindings/python/examples/group.py)
example.

A new group session is created by calling `local_app.create_session(...)` which returns a `SessionContext`
object containing both the session and a completion handle. The completion handle must be awaited to ensure
the session is fully established.

Key configuration parameters for `SessionConfig`:

- `session_type`: Set to `SessionType.GROUP` for group/multicast sessions.
- `enable_mls`: Set to `True` to enable MLS for end-to-end encryption.
- `max_retries`: Maximum number of retransmission attempts (upon missing ack) before notifying the application of delivery failure.
- `interval`: Duration to wait for an acknowledgment; if the ack is not received in time, a retry is triggered. If omitted / None, the session is unreliable (no retry/ack flow).
- `metadata`: Optional key-value pairs for session metadata.

After session creation, the moderator invites participants via `created_session.invite_async(invite_name)`.
Each invite call returns a completion handle that should be awaited to ensure the invitation completes.

### Implement Participants and Receive Messages

The group participants are implemented in a similar way, but they
do not create the session. They create the SLIM service instance and wait
for invites. Once they receive the invite, they can read and write on the shared channel.

```python
async def receive_loop(
    local_app: slim_bindings.App,
    created_session: slim_bindings.Session | None,
    session_ready: asyncio.Event,
    shared_session_container: list,
):
    """
    Receive messages for the bound session.

    Behavior:
      * If not moderator: wait for a new group session (listen_for_session_async()).
      * If moderator: reuse the created_session reference.
      * Loop forever until cancellation or an error occurs.
    """
    if created_session is None:
        print_formatted_text("Waiting for session...", style=custom_style)
        session = await local_app.listen_for_session_async(None)
    else:
        session = created_session

    # Make session available to other tasks
    shared_session_container[0] = session
    session_ready.set()

    # Get source and destination names for display
    source_name = session.source()

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

            # if the message metadata contains PUBLISH_TO this message is a reply
            # to a previous one. In this case we do not reply to avoid loops
            if "PUBLISH_TO" not in ctx.metadata:
                reply = f"message received by {source_name}"
                await session.publish_to_async(ctx, reply.encode(), None, ctx.metadata)
        except asyncio.CancelledError:
            # Graceful shutdown path (ctrl-c or program exit).
            break
        except Exception as e:
            # Break if session is closed, otherwise continue listening
            if "session closed" in str(e).lower():
                break
            continue
```

Each non-moderator participant listens for an incoming session using
`local_app.listen_for_session_async(None)`. The `None` parameter means wait indefinitely for a session.
This returns a session object containing metadata such as session ID, type, source name, and destination name.
The moderator already holds this information and therefore reuses the existing
`created_session` (see `session = created_session`).

Once a session is established, the function retrieves the source name using `session.source()` for display purposes.
Participants (including the moderator) then call `received_msg = await session.get_message_async(timeout=...)` to receive
messages. The returned `ReceivedMessage` object has two attributes: `context` (with source, destination, message type, and metadata)
and `payload` (the raw message bytes).

The function displays each received message showing the sender's name and payload. Additionally, it automatically sends
an acknowledgment reply using `session.publish_to_async(ctx, reply, None, metadata)`, unless the received message already
contains "PUBLISH_TO" in its metadata (indicating it's a reply), which prevents infinite reply loops.

### Publish Messages to the Session

All participants can publish messages on the shared channel:

```python
async def keyboard_loop(
    created_session: slim_bindings.Session,
    session_ready: asyncio.Event,
    shared_session_container: list[slim_bindings.Session],
    local_app: slim_bindings.App,
):
    """
    Interactive loop allowing participants to publish messages.

    Typing 'exit' or 'quit' (case-insensitive) terminates the loop.
    Typing 'remove NAME' removes a participant from the group
    Typing 'invite NAME' invites a participant to the group
    Each line is published to the group channel as UTF-8 bytes.
    """
    try:
        # 1. Initialize an async session
        prompt_session = PromptSession(style=custom_style)

        # Wait for the session to be established
        await session_ready.wait()

        session = shared_session_container[0]
        source_name = session.source()
        dest_name = session.destination()

        if created_session:
            print_formatted_text(
                f"Welcome to the group {dest_name}!\n"
                "Commands:\n"
                "  - Type a message to send it to the group\n"
                "  - 'remove NAME' to remove a participant\n"
                "  - 'invite NAME' to invite a participant\n"
                "  - 'exit' or 'quit' to leave the group",
                style=custom_style,
            )
        else:
            print_formatted_text(
                f"Welcome to the group {dest_name}!\n"
                "Commands:\n"
                "  - Type a message to send it to the group\n"
                "  - 'exit' or 'quit' to leave the group",
                style=custom_style,
            )

        while True:
            # Run blocking input() in a worker thread so we do not block the event loop.
            user_input = await prompt_session.prompt_async(f"{source_name} > ")

            if user_input.lower() in ("exit", "quit") and created_session:
                # Delete the session
                handle = await local_app.delete_session_async(
                    shared_session_container[0]
                )
                await handle.wait_async()
                break

            if user_input.lower().startswith("invite ") and created_session:
                invite_id = user_input[7:].strip()  # Skip "invite " (7 chars)
                await handle_invite(shared_session_container[0], invite_id)
                continue

            if user_input.lower().startswith("remove ") and created_session:
                remove_id = user_input[7:].strip()  # Skip "remove " (7 chars)
                await handle_remove(shared_session_container[0], remove_id)
                continue

            # Send message to the channel_name specified when creating the session.
            # As the session is group, all participants will receive it.
            await shared_session_container[0].publish_async(
                user_input.encode(), None, None
            )
    except KeyboardInterrupt:
        # Handle Ctrl+C gracefully
        pass
    except asyncio.CancelledError:
        # Handle task cancellation gracefully
        pass
    except Exception as e:
        print_formatted_text(f"-> Error sending message: {e}")
```

Messages are sent using `session.publish_async(payload, payload_type, metadata)`.
The payload is the message bytes, while payload_type and metadata are optional.
There is no explicit destination because the group channel was fixed at session creation and delivery
fans out to all participants.

The `keyboard_loop` function provides different interfaces depending on whether the application is a moderator or a regular participant:

- Moderators (who created the session) have additional commands:

  - `invite NAME` - Dynamically invite a new participant to the group using the `handle_invite()` helper
  - `remove NAME` - Remove a participant from the group using the `handle_remove()` helper
  - `exit` or `quit` - Delete the session, which notifies all participants

- Regular participants can:

  - Type messages to broadcast to the group
  - `exit` or `quit` - Leave the group (local session only)

When a user types `exit` or `quit` as a moderator, the application calls `local_app.delete_session_async()`
which returns a completion handle that must be awaited to ensure proper session cleanup before
terminating the loop. When the moderator closes the session,
all other participants are automatically notified, causing their receive loops to terminate
and their sessions to close gracefully. If the session closure is initiated by a participant,
only its local session is closed.

### Run the Group Communication Example

Now we will show how to run a new group session and
how to enable group communication on top of SLIM. The full code can be found in
[group.py](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/bindings/python/examples/group.py)
in the SLIM repo. To run the example, follow the steps listed here:

#### Run SLIM

As all members of the group are communicating via a SLIM network, we can set
up a SLIM instance representing the SLIM network. We use the pre-built
docker image for this purpose.

First execute this command to create the SLIM configuration file. Details about
the configuration can be found in the SLIM repository documentation.

```bash
cat << EOF > ./config.yaml
tracing:
  log_level: info
  display_thread_names: true
  display_thread_ids: true

runtime:
  n_cores: 0
  thread_name: "slim-data-plane"
  drain_timeout: 10s

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "0.0.0.0:46357"
          tls:
            insecure: true

      clients: []
    controller:
      servers: []
EOF
```

This configuration starts a SLIM instance with a server listening on port
46357, without TLS encryption for simplicity. Messages are still encrypted
using the MLS protocol, but the connections between SLIM nodes do not use TLS.
In a production environment, it is recommended to always use TLS and configure
proper authentication and authorization mechanisms.

You can run the SLIM instance using Docker:

```bash
docker run -it \
    -v ./config.yaml:/config.yaml -p 46357:46357 \
    ghcr.io/agntcy/slim:1.0.0 /slim --config /config.yaml
```

If everything goes fine, you should see an output like this one:

```bash
2026-01-28T15:03:38.408128Z  INFO main ThreadId(01) application_lifecycle: slim: 52: Runtime started
2026-01-28T15:03:38.414090Z  INFO main ThreadId(01) application_lifecycle: slim_service::service: 338: dataplane server started endpoint=0.0.0.0:46357
2026-01-28T15:03:38.414813Z  INFO main ThreadId(01) application_lifecycle: slim_service::service: 225: no controller configuration provided, skipping controller startup
2026-01-28T15:03:38.414823Z  INFO main ThreadId(01) application_lifecycle: slim: 65: service started service=slim/0
...
```

#### Start the Participants

In this example, we use two participants: `agntcy/ns/client-1` and `agntcy/ns/client-2`.

First, clone the SLIM repository and install the dependencies:

```bash
git clone --branch slim-v1.1.0 https://github.com/agntcy/slim.git
cd slim/data-plane/bindings/python
task python:bindings:build
task python:bindings:install-examples
```

Now run these commands in two different terminals from the `slim/data-plane/bindings/python` directory:

```bash
task python:example:group:client-1
```

```bash
task python:example:group:client-2
```

This starts two participants authenticated with a shared secret.
The output of these commands should look like this:

```bash
2026-01-28T15:20:54.595284Z  INFO slim slim_service::service: 402: client connected endpoint=http://127.0.0.1:46357 conn_id=0
Using shared-secret authentication.
12628940569466571367                         Created app
Waiting for session...
```

#### Create the Group

Run the moderator application to create the session and invite the two
participants. In another terminal, from the `slim/data-plane/bindings/python` directory, run:

```bash
task python:example:group:moderator
```

The result should look like:

```bash
2026-01-28T15:22:08.907149Z  INFO slim slim_service::service: 402: client connected endpoint=http://127.0.0.1:46357 conn_id=0
Using shared-secret authentication.
2704185522498177164                          Created app
Creating new group session (moderator)... agntcy/ns/moderator/ffffffffffffffff
agntcy/ns/moderator -> add agntcy/ns/client-1/ffffffffffffffff to the group
agntcy/ns/moderator -> add agntcy/ns/client-2/ffffffffffffffff to the group
Welcome to the group agntcy/ns/chat/ffffffffffffffff!
Commands:
  - Type a message to send it to the group
  - 'remove NAME' to remove a participant
  - 'invite NAME' to invite a participant
  - 'exit' or 'quit' to leave the group
agntcy/ns/moderator/25873267c342088c >
```

Now `client-1` and `client-2` are invited to the group, so on both of their terminals you should
be able to see a welcome message such as:

```bash
Welcome to the group agntcy/ns/chat/ffffffffffffffff!
Commands:
  - Type a message to send it to the group
  - 'exit' or 'quit' to leave the group
agntcy/ns/client-1/af43028974930a67 >
```

At this point, you can write messages from any terminal and they will be received by all other group participants.

Writing 'exit' or 'quit' from the moderator will close all the applications.
You can also remove and add participants by using the `remove` and `invite` commands from the moderator.

## Group Communication Using the SLIM Controller

Previously, we saw how to run group communication using the Python bindings with an in-application moderator.
This participant creates the group session and invites all other participants.
In this section, we describe how to create and orchestrate a group using the SLIM Controller, and we show how all
these functions can be delegated to the controller. We reuse the same group example code in this section as well.

### Application Differences

With the controller, you do not need to set up a moderator in your application. All participants can be run as we did for `client-1` and `client-2` in the previous examples. In code, this means you can avoid creating a new group session (using `local_app.create_session`) and the invitation loop. You only need to implement the `receive_loop` where the application waits for new sessions. This greatly simplifies your code.

### Run the Group Communication Example

Now we will show how to set up a group using the SLIM Controller. The reference code for the
application is still [group.py](https://github.com/agntcy/slim/tree/slim-v1.1.0/data-plane/bindings/python/examples/group.py). To run this example, follow the steps listed here.

#### Run the SLIM Controller

First, start the SLIM Controller. Full details are in the [SLIM Controller](./slim-controller.md) documentation; here we reproduce the minimal setup. Create a configuration file:

```bash
cat << EOF > ./config-controller.yaml
northbound:
  httpHost: 0.0.0.0
  httpPort: 50051
  logging:
    level: INFO

southbound:
  httpHost: 0.0.0.0
  httpPort: 50052
  logging:
    level: INFO

reconciler:
  maxRequeues: 15
  maxNumOfParallelReconciles: 1000

logging:
  level: INFO

database:
  filePath: /db/controlplane.db
EOF
```

This config defines two APIs exposed by the controller:

- Northbound API: used by an operator (e.g. via slimctl) to configure channels and participants, as well as the SLIM network.
- Southbound API: used by SLIM nodes to synchronize with the controller.

Start the controller with Docker:

```bash
docker run -it \
    -v ./config-controller.yaml:/config.yaml -v .:/db -p 50051:50051 -p 50052:50052 \
    ghcr.io/agntcy/slim/control-plane:1.0.0 --config /config.yaml
```

If everything goes fine, you should see an output like this:

```bash
2026-01-28T15:26:53Z INF Starting Route Reconciler thread_name=reconciler
2026-01-28T15:26:53Z INF Northbound API Service is listening on [::]:50051
2026-01-28T15:26:53Z INF Southbound API Service is Listening on [::]:50052
```

#### Run the SLIM Node

With the controller running, start a SLIM node configured to talk to it over the Southbound API. This node config includes two additional settings compared to the file from the previous section:

- A controller client used to connect to the Southbound API running on port 50052.
- A shared secret token provider that will be used by the SLIM node to send messages over the SLIM network. As with the normal application, you can use a shared secret or a proper JWT.

Create the `config-slim.yaml` for the node using the command below. We use the `host.docker.internal` endpoint to reach the controller from inside the Docker container via the host.

```bash
cat << EOF > ./config-slim.yaml
tracing:
  log_level: info
  display_thread_names: true
  display_thread_ids: true

runtime:
  n_cores: 0
  thread_name: "slim-data-plane"
  drain_timeout: 10s

services:
  slim/0:
    dataplane:
      servers:
        - endpoint: "0.0.0.0:46357"
          tls:
            insecure: true

      clients: []
    controller:
      servers: []
      clients:
        - endpoint: "http://host.docker.internal:50052"
          tls:
            insecure: true
      token_provider:
        type: shared_secret
        id: "slim/0"
        data: "very-long-shared-secret-value-0123456789abcdef"
EOF
```

This starts a SLIM node that connects to the controller:

```bash
docker run -it \
    -v ./config-slim.yaml:/config.yaml -p 46357:46357 \
    ghcr.io/agntcy/slim:1.0.0 /slim --config /config.yaml
```

If everything goes fine, you should see an output like this one:

```bash
2026-01-28T15:40:45.189063Z  INFO main ThreadId(01) application_lifecycle: slim: 52: Runtime started
2026-01-28T15:40:45.189274Z  INFO main ThreadId(01) application_lifecycle: slim_service::service: 338: dataplane server started endpoint=0.0.0.0:46357
2026-01-28T15:40:45.189348Z  INFO main ThreadId(01) application_lifecycle: slim_controller::service: 1634: connecting to control plane config.endpoint=http://host.docker.internal:50052
2026-01-28T15:40:45.192773Z  INFO slim-data-plane ThreadId(03) slim_controller::service: 1518: connected to control plane endpoint=http://host.docker.internal:50052
2026-01-28T15:40:45.192777Z  INFO            main ThreadId(01) application_lifecycle: slim: 65: service started service=slim/0
2026-01-28T15:40:45.196599Z  INFO slim-data-plane ThreadId(04) slim_controller::service: 890: Processed ConfigurationCommand connections=0 subscriptions_to_set=0 subscriptions_to_del=0
...
```

On the Controller side, you can see that the new node registers with the controller. The
output should be similar to this:

```bash
2026-01-28T15:40:45Z INF Registering node with ID: slim/0 svc=southbound
2026-01-28T15:40:45Z INF Connection details: [endpoint: 192.168.65.1:46357] nodeID=slim/0 svc=southbound
2026-01-28T15:40:45Z INF Create generic routes for node node_id=slim/0 service=RouteService
2026-01-28T15:40:45Z INF Sending routes to registered node slim/0 node_id=slim/0
2026-01-28T15:40:45Z INF Sending configuration command to registered node connections_count=0 message_id=95c75638-aa2d-4043-8f27-cb26f453716e node_id=slim/0 subscriptions_count=0 subscriptions_to_delete_count=0
2026-01-28T15:40:45Z INF Configuration command processing completed node_id=slim/0 original_message_id=95c75638-aa2d-4043-8f27-cb26f453716e
```

#### Run the Participants

Because the controller manages the group lifecycle, no participant needs to be designated as moderator in code. Every application instance just waits for a session invite. In three separate terminals, from the folder
`data-plane/bindings/python/examples` run:

```bash
uv run slim-bindings-group \
    --local agntcy/ns/client-1 \
    --shared-secret "very-long-shared-secret-value-0123456789abcdef"
```

```bash
uv run slim-bindings-group \
    --local agntcy/ns/client-2 \
    --shared-secret "very-long-shared-secret-value-0123456789abcdef"
```

```bash
uv run slim-bindings-group \
    --local agntcy/ns/client-3 \
    --shared-secret "very-long-shared-secret-value-0123456789abcdef"
```

Each terminal should show output similar to:

```bash
2026-01-28T15:44:59.125043Z  INFO slim slim_service::service: 402: client connected endpoint=http://127.0.0.1:46357 conn_id=0
Using shared-secret authentication.
10494544672403736104                         Created app
Waiting for session..
```

At this point all applications are waiting for a new session.

#### Manage the Group with slimctl

Use `slimctl` (see [SLIM Controller](./slim-controller.md)) to send administrative commands to the controller.

First, you need to run `slimctl`. To install it see the [slimctl documentation](./slim-controller.md#installing-slimctl).

To verify that `slimctl` was downloaded successfully, run the following command:

```bash
slimctl version
```

##### Create the Group

Select any running participant to be the initial member of the group. This participant acts as the logical
moderator of the channel, similar to the Python bindings example. However, you don't
need to handle this explicitly in the code. Run the following command to create the channel:

```bash
slimctl controller channel create moderators=agntcy/ns/client-1/10494544672403736104 
```

The full name of the application can be taken from the output in the console. The value
`10494544672403736104` is the actual ID of the `client-1` application returned by
SLIM and is visible in the logs. In your case, this value will be different.

The expected response from `slimctl` is:

```bash
Received response: agntcy/ns/hDxc8CKpElJUfTTief
```

The value `agntcy/ns/hDxc8CKpElJUfTTief` is the channel (or group) identifier (name) that must be used in subsequent commands.

On the application side, `client-1` was added to the session, so you should see
something like this:

```bash
Welcome to the group agntcy/ns/hDxc8CKpElJUfTTief/ffffffffffffffff!
Commands:
  - Type a message to send it to the group
  - 'exit' or 'quit' to leave the group
agntcy/ns/client-1/91a41cc6ee17c628 >
```

##### Add Participants

Now that the new group is created, add the additional participants `client-2` and `client-3` using the following `slimctl` commands:

```bash
slimctl controller participant add -c agntcy/ns/xyIGhc2igNGmkeBDlZ agntcy/ns/client-2
slimctl controller participant add -c agntcy/ns/xyIGhc2igNGmkeBDlZ agntcy/ns/client-3
```

The expected `slimctl` output is:

```bash
Adding participant to channel ID agntcy/ns/hDxc8CKpElJUfTTief: agntcy/ns/client-2
Participant added successfully to channel ID agntcy/ns/hDxc8CKpElJUfTTief: agntcy/ns/client-2
```

Now all the participants are part of the same group, and so each client log should show that the join was successful:

```bash
Welcome to the group agntcy/ns/hDxc8CKpElJUfTTief/ffffffffffffffff!
Commands:
  - Type a message to send it to the group
  - 'exit' or 'quit' to leave the group
agntcy/ns/client-2/e9b95e5edaee3e2c >
```

At this point, every member can send messages, and they will be received by all the other participants.

##### Remove a Participant

To remove one of the participants from the channel, run the following command:

```bash
slimctl controller participant delete -c agntcy/ns/xyIGhc2igNGmkeBDlZ agntcy/ns/client-3
```

The `slimctl` expected output is this:

```bash
Deleting participant from channel ID agntcy/ns/hDxc8CKpElJUfTTief: agntcy/ns/client-3
Participant deleted successfully from channel ID agntcy/ns/hDxc8CKpElJUfTTief: agntcy/ns/client-3
```

The application on `client-3` exits because the session related to the group was closed, which breaks the
receive loop in the Python code.

##### Delete channel

To delete the channel, run the following command:

```bash
slimctl controller channel delete agntcy/ns/xyIGhc2igNGmkeBDlZ
```

The `slimctl` output is this:

```bash
Channel deleted successfully with ID: agntcy/ns/hDxc8CKpElJUfTTief
```

All applications connected to the group stop as their receive loops terminate.
