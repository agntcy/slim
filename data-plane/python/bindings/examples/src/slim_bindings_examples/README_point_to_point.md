# Point-to-Point Example with SLIM Python Bindings

TODO: check this file

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

### 1. Start the SLIM server

Before running the point-to-point example, you need a running SLIM server. You can start a local SLIM server using the Taskfile:

```bash
task python:example:server
```

This will start the SLIM server on `127.0.0.1:46357` by default.

### 2. Run the point-to-point example

The recommended way to run the example is via the Taskfile command. See the Taskfile for available roles (e.g., ff:alice, ff:bob, rr:responder, rr:requester) and argument details.

**Manual usage:**

You can also run the script directly:

```bash
python point_to_point.py --local <LOCAL_ID> --slim <SLIM_CONFIG> \
    --remote <REMOTE_ID> --message "hello" --iterations 3 --sticky [OPTIONS]
```

## Notes
- Use the `--sticky` flag to ensure the session always connects to the same remote endpoint.
- Use the `--enable-mls` flag to enable secure messaging with MLS.
- If no `--message` is provided, the script will act as a receiver and reply to incoming messages.
- See the code comments for further details on each step.
