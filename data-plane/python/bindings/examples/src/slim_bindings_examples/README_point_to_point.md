
# Point-to-Point Example with SLIM Python Bindings

This example shows how to build point‑to‑point communication flows
with the SLIM Python bindings. You can run a sender (Bob) and one
or more receivers (Alice instances) and observe differences in routing,
delivery semantics, and (optionally) secure messaging with Messaging Layer
Security (MLS).

## Features

- PointToPoint sessions (sticky peer selection after discovery)
- Automatic echo reply example from receiver
- Optional secure PointToPoint with MLS (`--enable-mls`)

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

### 2. Sender vs Receiver

The example process acts as a sender when you pass `--message`. Otherwise it
behaves as a long‑running receiver that waits for sessions initiated by
senders and echoes messages back.

Relevant options (see [Taskfile.yaml](../../Taskfile.yaml)):
- `--message`: triggers sender mode
- `--iterations`: how many messages to send (default 10 in Taskfile examples)
- `--enable-mls`: enable MLS
- `--remote`: the target application name (required in sender mode)

### PointToPoint session (with optional MLS)

In a PointToPoint session, a Discovery mechanism selects one target instance and
all subsequent traffic is pinned to that peer for the session lifetime.

```python
    # Convert the remote ID string into a Name.
    remote_name = split_id(remote)
    # Establish routing so outbound publishes know the remote destination.
    await local_app.set_route(remote_name)

    config = slim_bindings.SessionConfiguration.PointToPoint(
        max_retries=5,
        timeout=datetime.timedelta(seconds=5),
        mls_enabled=enable_mls,
    )
    session, handle = await local_app.create_session(remote_name, config)
    await handle
```

Reliability parameters:
- `max_retries`: maximum retransmission attempts for lost messages.
- `timeout`: interval (timedelta) between retransmission attempts.

When MLS is enabled (`--enable-mls`), payloads are protected using the MLS
protocol; only session members can decrypt and authenticate messages.

The `local_app.create_session(...)` create the session and returns an handler `handle`. 
The `await handle` guarantees that the session is fully established when it returns.  

### 3. Sender publish & response handling

In sender mode the example loops for `iterations` times, publishing and then
waiting for a reply (Alice echoes it). Logic summary:

```python
    for i in range(iterations):
        try:
            await session.publish(message.encode())
            format_message_print(
                f"{instance}",
                f"Sent message {message} - {i + 1}/{iterations}:",
            )
            # Wait for reply from remote peer.
            _msg_ctx, reply = await session.get_message()
            format_message_print(
                f"{instance}",
                f"received (from session {session.id}): {reply.decode()}",
            )
        except Exception as e:
            # Surface an error but continue attempts (simple resilience).
            format_message_print(f"{instance}", f"error: {e}")
        # Basic pacing so output remains readable.
        await asyncio.sleep(1)
```

### 4. Receiver session & echo loop

Without `--message`, the process waits for inbound sessions:

```python
    # Block until a remote peer initiates a session to us.
    session = await local_app.listen_for_session()
    format_message_print(f"{instance}", f"new session {session.id}")

    async def session_loop(sess: slim_bindings.Session):
        """
        Inner loop for a single inbound session:
            * Receive messages until the session is closed or an error occurs.
            * Echo each message back using publish_to.
        """
        while True:
            try:
                msg_ctx, payload = await sess.get_message()
            except Exception:
                # Session likely closed or transport broken.
                break
            text = payload.decode()
            format_message_print(f"{instance}", f"received: {text}")
            # Echo reply with appended instance identifier.
            await sess.publish_to(msg_ctx, f"{text} from {instance}".encode())

    # Launch a dedicated task to handle this session (allow multiple).
    asyncio.create_task(session_loop(session))
```

Key APIs:
- `listen_for_session()`: blocks until a remote sender establishes a session.
- `get_message()`: returns `(context, payload)`.
- `publish_to(msg_ctx, data)`: reply directly to sender context.

This model supports multiple concurrent sessions (each gets its own task).


## Usage

Use the Taskfile targets for reproducible runs. See
[Taskfile.yaml](../../Taskfile.yaml) for full command reference.

### 1. Start the SLIM server

Start the local SLIM server:

```bash
task python:example:server
```

Default endpoint: `127.0.0.1:46357`.

### 2. Run Alice (receiver)

Open a terminal and run:

```bash
task python:example:p2p:alice
```

Alice waits for sessions and echoes each received message with its own ID.

### 3. Run Bob (sender)

In another terminal run one of:

#### a) PointToPoint (no MLS)

```bash
task python:example:p2p:no-mls:bob
```

#### b) PointToPoint with MLS

```bash
task python:example:p2p:mls:bob
```

Each command sends the configured `--message` (default in Taskfile: "hey there")
for the default number of iterations and prints echoed replies.
