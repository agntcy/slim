
# Point-to-Point Example with SLIM Python Bindings

This example demonstrates how to use the SLIM Python bindings to create and
manage point-to-point sessions (Anycast and Unicast) between distributed
application instances. The script allows you to send and receive messages
directly between two endpoints, with optional security using Messaging Layer
Security (MLS).

## Features

- Create point-to-point sessions (Anycast or Unicast)
- Send and receive messages between two endpoints
- Optionally enable Messaging Layer Security (MLS) for secure communication

## How It Works

### 1. Create the local application

The script initializes a local SLIM application instance using several
configuration options:

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

### 2. Create a Session

The script can act as either a sender or a receiver, depending on whether the
`--message` flag is provided. If the message is set, the application runs as a
sender and sets up the session according to the parameters specified. You can
see the [Taskfile.yaml](../../Taskfile.yaml) file to check all the available
parameters.


### Anycast session
If the unicast option is not selected and MLS is disabled, the session created
by the sender is an Anycast session. This means that if there are multiple
instances of the same app running, each message can be routed to a different
application instance, providing natural load balancing.

```python
    session = await local_app.create_session(
        slim_bindings.PySessionConfiguration.Anycast()  # type: ignore
    )
```


In this example, no retransmission parameter is set. To see all the parameters
and for a more in-depth explanation of how the Anycast session works, please
check [SESSION.md](../../../SESSION.md).


### Unicast session
If the unicast option is set or MLS is enabled, a Unicast session is created.
This session first runs a discovery phase to find one instance of an
application with a certain name, and after this, all messages are always
routed to the same endpoint for the duration of the session.

```python
    session = await local_app.create_session(
        slim_bindings.PySessionConfiguration.Unicast(  # type: ignore
            max_retries=5,
            timeout=datetime.timedelta(seconds=5),
            mls_enabled=enable_mls,
        )
    )
```

In this example, we set the parameters for retransmissions, and MLS can be
enabled or not depending on the `mls_enabled` flag. Check
[SESSION.md](../../../SESSION.md) for more information on all the parameters
and to see how the Unicast session works.


TODO: receive part depends on the new code


## Usage

The recommended way to run the point-to-point example is via the Taskfile commands.
If you want to see all the command flags, check the
[Taskfile.yaml](../../Taskfile.yaml) file.

### 1. Start the SLIM server

Before running the point-to-point example, you need a running SLIM server.
 You can start a local SLIM server using the Taskfile:

```bash
task python:example:server
```

This will start the SLIM server on `127.0.0.1:46357` by default.

### 2. Run Alice (receiver)


Open a terminal and run:

```bash
task python:example:p2p:alice
```

Alice will listen for messages and echo them back to the sender. You can run
multiple instances of Alice if you want to test the differences between
Anycast and Unicast.

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

Each command will send a message to Alice using the specified session type
and security options.
