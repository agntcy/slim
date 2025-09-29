
# Point-to-Point Example with SLIM Python Bindings

This example shows how to build point‑to‑point communication flows (Anycast
and Unicast) with the SLIM Python bindings. You can run a sender (Bob) and one
or more receivers (Alice instances) and observe differences in routing,
delivery semantics, and (optionally) secure messaging with Messaging Layer
Security (MLS).

## Features

- Anycast sessions (per-message dynamic target selection / load distribution)
- Unicast sessions (sticky peer selection after discovery)
- Automatic echo reply example from receiver
- Optional secure Unicast with MLS (`--enable-mls`)

## How It Works

### 1. Create the local application

First we construct a local SLIM application instance:

```python
local_app = await create_local_app(
    local,
    slim,
    enable_opentelemetry=enable_opentelemetry,
    shared_secret=shared_secret,
    jwt=jwt,
    spire_trust_bundle=spire_trust_bundle,
    audience=audience,
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
If `jwt`, `spire-trust-bundle` and `audience` are not, `shared_secret` must be set (only
recommended for local testing / example, not production).

### 2. Sender vs Receiver

The example process acts as a sender when you pass `--message`. Otherwise it
behaves as a long‑running receiver that waits for sessions initiated by
senders and echoes messages back.

Relevant options (see [Taskfile.yaml](../../Taskfile.yaml)):
- `--message`: triggers sender mode
- `--iterations`: how many messages to send (default 10 in Taskfile examples)
- `--enable-mls`: enable MLS (forces Unicast semantics)
- `--remote`: the target application name (required in sender mode)


### Anycast session

If neither the `unicast` flag nor MLS is enabled, an Anycast session is used.
Each message may be directed to potentially different healthy instances of
the destination name (stateless load distribution).

```python
session = await local_app.create_session(
    slim_bindings.PySessionConfiguration.Anycast()  # type: ignore
)
```

In this mode the example uses `publish_with_destination` to explicitly name
the destination for each message while still allowing the underlying routing
to select an instance.

See [SESSION.md](../../../SESSION.md) for advanced Anycast behavior and
parameters.


### Unicast session (with optional MLS)

If the sender chooses the unicast path (explicit unicast example) or enables
MLS, a Unicast session is created. Discovery selects one target instance and
all subsequent traffic is pinned to that peer for the session lifetime.

```python
remote_name = split_id(remote)
await local_app.set_route(remote_name)
session = await local_app.create_session(
    slim_bindings.PySessionConfiguration.Unicast(  # type: ignore
        unicast_name=remote_name,
        max_retries=5,
        timeout=datetime.timedelta(seconds=5),
        mls_enabled=enable_mls,
    )
)
```

Reliability parameters:
- `max_retries`: maximum retransmission attempts for lost messages.
- `timeout`: interval (timedelta) between retransmission attempts.

When MLS is enabled (`--enable-mls`), payloads are protected using the MLS
protocol; only session members can decrypt and authenticate messages.

### 3. Sender publish & response handling

In sender mode the example loops for `iterations` times, publishing and then
waiting for a reply (Alice echoes it). Logic summary:

```python
for i in range(iterations):
    if unicast or enable_mls:
        await session.publish(message.encode())
    else:
        await session.publish_with_destination(message.encode(), remote_name)
    _ctx, reply = await session.get_message()
    print("received reply", reply.decode())
```

Anycast uses `publish_with_destination` to specify the logical destination
name; Unicast (and Unicast+MLS) uses `publish` since the peer is already
bound.

### 4. Receiver session & echo loop

Without `--message`, the process waits for inbound sessions:

```python
while True:
    session = await local_app.listen_for_session()
    async def session_loop(session):
        while True:
            msg_ctx, payload = await session.get_message()
            text = payload.decode()
            await session.publish_to(msg_ctx, f"{text} from {local_app.id}".encode())
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
Run multiple Alice instances to observe Anycast load distribution.

### 3. Run Bob (sender)

In another terminal run one of:

#### a) Anycast (no MLS)

```bash
task python:example:p2p:anycast:bob
```

#### b) Unicast (no MLS)

```bash
task python:example:p2p:unicast:no-mls:bob
```

#### c) Unicast with MLS

```bash
task python:example:p2p:unicast:mls:bob
```

Each command sends the configured `--message` (default in Taskfile: "hey there")
for the default number of iterations and prints echoed replies.
