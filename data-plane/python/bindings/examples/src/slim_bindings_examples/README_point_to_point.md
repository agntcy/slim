# Point-to-Point Example with SLIM Python Bindings

This example demonstrates how to use the SLIM Python bindings to create and manage point-to-point (unicast and anycast) sessions between distributed application instances. The script allows you to send and receive messages directly between two endpoints, with optional session stickiness and security.

## Features
- Create point-to-point sessions (unicast or anycast)
- Send and receive messages between two endpoints
- Optionally enable Messaging Layer Security (MLS) for secure communication
- Support for sticky sessions (always connect to the same endpoint)
- Configurable number of message iterations

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

See the multicast example README for a detailed explanation of these options.

### 2. Send or receive messages

- If the `--message` flag is provided, the script acts as a sender:
  - It creates a route to the remote endpoint.
  - It creates a point-to-point session (sticky if `--sticky` or `--enable-mls` is set).
  - It sends the message the specified number of times (`--iterations`).
  - It waits for a reply after each message.

```python
if sticky or enable_mls:
    session = await local_app.create_session(
        slim_bindings.PySessionConfiguration.PointToPoint(
            max_retries=5,
            timeout=datetime.timedelta(seconds=5),
            sticky=True,
            mls_enabled=enable_mls,
        )
    )
else:
    session = await local_app.create_session(
        slim_bindings.PySessionConfiguration.PointToPoint()
    )

for i in range(0, iterations):
    await local_app.publish(session, message.encode(), remote_name)
    # ... wait for reply ...
```

- If no `--message` is provided, the script acts as a receiver:
  - It waits for a new session to be established.
  - It receives messages and replies to each one.

```python
while True:
    session_info, _ = await local_app.receive()
    async def background_task(session_id):
        while True:
            session, msg = await local_app.receive(session=session_id)
            # ... process and reply ...
    asyncio.create_task(background_task(session_info.id))
```

## Command-Line Options

- `--local`: Local application identity (string, required)
- `--remote`: Remote application identity (string, required for sending)
- `--slim`: SLIM node configuration (JSON or dict)
- `--message`: Message to send (if omitted, acts as receiver)
- `--iterations`: Number of messages to send (default: 2)
- `--sticky`: Enable sticky sessions (always connect to the same endpoint)
- `--enable-mls`: Enable Messaging Layer Security (optional)
- `--shared-secret`, `--jwt`, `--bundle`, `--audience`: Security options (optional)


## Usage

The recommended way to run the point-to-point example is via the Taskfile commands. If you want to see all the command flags, check the [Taskfile.yaml](../../Taskfile.yaml) file.

### 1. Start the SLIM server

Before running the point-to-point example, you need a running SLIM server. You can start a local SLIM server using the Taskfile:

```bash
task python:example:server
```

This will start the SLIM server on `127.0.0.1:46357` by default.

### 2. Run Alice (receiver)

Open a terminal and run:

```bash
task python:example:p2p:alice
```

Alice will listen for messages and echo them back to the sender.
You can run multiple instances of Alice if you want to test the differences
between Anycast and Unicast.

### 3. Run Bob (sender)

In a separate terminal, you can run Bob in different modes:

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

Each command will send a message to Alice using the specified session type and security options.
