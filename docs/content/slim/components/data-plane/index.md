# SLIM Data Plane

The [SLIM](../../../index.md) Data Plane implements an efficient message routing and
delivery system between applications.

## Client and Channel Naming

SLIM uses a hierarchical naming scheme to identify all endpoints:

```text
org/namespace/service/client
```

Messages can be delivered to a single specific instance (unicast) or to any
available instance of a service (anycast). Channels use the same naming
structure for many-to-many group communication.

For full details on the naming scheme, see [Naming](../../architecture/naming.md).

## SLIM Sessions Layer

The SLIM platform includes a session layer that connects application frameworks
to the underlying SLIM messaging infrastructure. This layer provides a simple
interface that abstracts the complexity of secure messaging and message
distribution from the application.

The session layer offers several functionalities:

- **Security**: All messages in SLIM are encrypted by default using the [MLS
  protocol](https://www.rfc-editor.org/rfc/rfc9420.html), which guarantees
  end-to-end encryption even when messages traverse intermediate nodes where TLS
  connections are terminated. The session layer is responsible for MLS group
  creation and updates, as well as message encryption and decryption.
- **Channel Management**: The session layer enables clients to be invited to or
  removed from a channel as needed.
- **Message Delivery**: The session layer abstracts message passing between
  applications and the SLIM message distribution network. It handles message
  formatting, routing, and delivery confirmation, while providing simple send
  and receive primitives to applications.

The session layer offers two primary APIs for establishing new sessions:

- **Point-to-Point**: Facilitates point-to-point communication with a specific service
  instance. This session performs a discovery phase to bind the session
  to a single instance; all subsequent messages in the session are sent to that
  same endpoint.

- **Group**: Supports many-to-many communication over a named channel.
Every message sent to the channel is delivered to all current participants.

For more information about each session type, see the
[SLIM session](../../architecture/session.md) documentation.
