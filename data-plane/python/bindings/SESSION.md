
# SLIM Sessions

This document explains the SLIM session layer and the three supported session
types. It helps you pick the right pattern, understand reliability and security
trade‑offs, and shows concrete Python usage examples.

---
## Quick Reference

| Type     | Pattern        | Reliability            | MLS | Primary Uses                          | Avoid When                                 |
|----------|----------------|------------------------|-----|----------------------------------------|--------------------------------------------|
| Anycast  | 1:1 stateless  | Best‑effort or retries | No  | Load balance, idempotent RPC, fan‑out  | Need per‑peer state or E2E encryption      |
| Unicast  | 1:1 stateful   | Optional acks + retries| Yes | Stateful convo, sticky peer, secure P2P| Need broadcast / many recipients           |
| Multicast| N:N channel    | Optional acks + retries| Yes | Group chat, pub/sub, coordination      | Need strict single recipient semantics     |

Key takeaways:
* Pick Anycast for simple stateless request distribution.
* Pick Unicast when you must bind to one specific instance and optionally
    secure with MLS.
* Pick Multicast when many peers need to exchange messages over the same shared
    channel.

---
## Table of Contents
1. [Anycast](#anycast)
2. [Unicast](#unicast)
3. [Multicast](#multicast)
---
## Anycast


The anycast session enables point-to-point (1:1) communication where each
message sent to a service is delivered to only one of its available instances.
This pattern provides natural load balancing and redundancy for stateless
services. Anycast is best suited for distributing requests across multiple
instances without maintaining session state.

Let's see an example of the communication pattern in the anycast session using
the sequence diagram below. The anycast session sends each message to a service
(e.g., App-B) and the message is delivered to only one of its available
instances (e.g., App-B/1 or App-B/2). The SLIM Node dynamically routes each
message to one of the running instances, so consecutive messages may be
delivered to different endpoints.

If reliability is enabled, the sender expects an acknowledgment (Ack) for every
message sent. This confirms successful delivery, even though the specific
instance may vary per message.

The diagram below illustrates two consecutive messages from App-A to the
service agntcy/ns/App-B. The first message is delivered to App-B/1, the second
to App-B/2. Each delivery is followed by an Ack:

```mermaid
sequenceDiagram
    autonumber

    participant App-A
    participant SLIM Node
    participant App-B/1
    participant App-B/2

    App-A->>SLIM Node: Message to agntcy/ns/App-B
    SLIM Node->>App-B/1: Message to agntcy/ns/App-B
    App-B/1->>SLIM Node: Ack
    SLIM Node->>App-A: Ack

    App-A->>SLIM Node: Message to agntcy/ns/App-B
    SLIM Node->>App-B/2: Message to agntcy/ns/App-B
    App-B/2->>SLIM Node: Ack
    SLIM Node->>App-A: Ack
```

Note: Anycast sessions are stateless and do not allow persistent per‑recipient
state. Consequently, Messaging Layer Security (MLS) cannot be enabled. If MLS
is required for point‑to‑point communication, use a Unicast session instead.


### Create an Anycast Session

Using the SLIM Python bindings, you can create an Anycast session as follows:

```python
# Assume local_app is an initialized application instance
session = await local_app.create_session(
    slim_bindings.PySessionConfiguration.Anycast(
        max_retries=5,  # Retries before giving up
        timeout=datetime.timedelta(seconds=5),  # Wait per attempt for Ack 
                                                # (omit for best-effort)
    )
)
```

Parameters:
* `max_retries` (optional, int): Number of retry attempts if an Ack is not
    received.
* `timeout` (optional, timedelta): How long to wait for an Ack before retrying.
    If omitted the session is best‑effort

If timeout is not provided the session is best‑effort (unreliable): lost messages
are not retransmitted.

### Sending and Replying in Anycast
Anycast sessions are not pinned to a single remote instance. Each outgoing
message must therefore specify a destination application name. Use
`publish_with_destination` for sends. The plain `publish` API is only valid for
sessions that already have an implicit peer or channel (Unicast / Multicast).

```python
remote_name = split_id("agntcy/ns/App-B")  # Resolve target service name
await session.publish_with_destination(b"hello", remote_name)

# This would raise for Anycast (no implicit peer):
# await session.publish(b"hello")
```

To reply to a received message, use `publish_to`, which routes a response back
to the original sender using the message context acquired via `get_message`:

```python
async def session_loop(session: slim_bindings.PySession):  # type: ignore
    while True:
        try:
            msg_ctx, payload = await session.get_message()
        except Exception:
            break  # Session likely closed
        text = payload.decode()
        format_message_print(f"{instance}", f"received: {text}")
        await session.publish_to(msg_ctx, f"{text} from {instance}".encode())
```

## Unicast

The Unicast session enables point‑to‑point communication with a specific
instance. Unlike Anycast (which re‑selects an instance each message), Unicast
performs a discovery phase to bind to one instance; all subsequent traffic in
the session targets that same endpoint. This enables stateful interactions and
session continuity. With reliability enabled each message must be Acked.

If MLS is enabled, the Unicast session establishes a two‑member MLS group after
discovery: invite → key package reply → MLS Welcome. This mirrors the
Multicast flow but with only two participants.


The diagram below illustrates a unicast session from App-A to agntcy/ns/App-B.
App-A first discovers an available instance (App-B/1), then performs the MLS
setup, and finally sends multiple messages to that same instance, each followed
by an Ack. If MLS is not enabled, the MLS setup is skipped:

```mermaid
sequenceDiagram
    autonumber

    participant App-A
    participant SLIM Node
    participant App-B/1
    participant App-B/2

    Note over App-A,App-B/2: Discovery
    App-A->>App-A: Init MLS state
    App-A->>SLIM Node: Discover agntcy/ns/App-B
    SLIM Node->>App-B/1: Discover agntcy/ns/App-B
    App-B/1->>SLIM Node: Discover Reply (agntcy/ns/App-B/1)
    SLIM Node->>App-A: Discover Reply (agntcy/ns/App-B/1)

    Note over App-A,App-B/2: Invite
    App-A->>SLIM Node: Invite agntcy/ns/App-B/1
    SLIM Node->>App-B/1: Invite agntcy/ns/App-B/1
    App-B/1->>App-B/1: Create new Unicast Session
    App-B/1->>SLIM Node: Invite Reply (MLS key package)
    SLIM Node->>App-A: Invite Reply (MLS key package)
    App-A->>App-A: Update MLS state

    Note over App-A,App-B/2: MLS setup
    App-A->>SLIM Node: MLS Welcome agntcy/ns/App-B/1
    SLIM Node->>App-B/1: MLS Welcome agntcy/ns/App-B/1
    App-B/1->>App-B/1: Init MLS state
    App-B/1->>SLIM Node: Ack(MLS Welcome)
    SLIM Node->>App-A: Ack(MLS Welcome)

    Note over App-A,App-B/2: Message exchange
    App-A->>SLIM Node: Message to agntcy/ns/App-B/1
    SLIM Node->>App-B/1: Message to agntcy/ns/App-B/1
    App-B/1->>SLIM Node: Ack
    SLIM Node->>App-A: Ack

    App-A->>SLIM Node: Message to agntcy/ns/App-B/1
    SLIM Node->>App-B/1: Message to agntcy/ns/App-B/1
    App-B/1->>SLIM Node: Ack
    SLIM Node->>App-A: Ack
```


### Create a Unicast Session

Using the SLIM Python bindings, you can create a Unicast session as follows:

```python
# Assume local_app is an initialized application instance
session = await local_app.create_session(
    slim_bindings.PySessionConfiguration.Unicast(
        unicast_name=remote_name,
        max_retries=5,
        timeout=datetime.timedelta(seconds=5),
        mls_enabled=True,  # Enable MLS for end-to-end security
    )
)
```

Parameters:
* `unicast_name` (required, PyName): Identifier of the remote participant
    instance.
* `max_retries` (optional, int): Retry attempts per message if Ack missing.
* `timeout` (optional, timedelta): Wait per attempt for an Ack before retry.
* `mls_enabled` (optional, bool): Enable end‑to‑end encryption (MLS).

If `timeout` is not set the session is best‑effort.

### Sending and Replying in Unicast
In Unicast the session is bound to a single remote instance after discovery, so
outbound messages use the implicit destination. Use `publish` for normal sends
and `publish_to` to reply using a previously received message context. Do not
use `publish_with_destination` (it will raise) because the peer is already
fixed.

```python
# Send a request
await session.publish(b"hello")

# Await reply from remote (pattern depends on your control loop)
msg_ctx, payload = await session.get_message()
print(payload.decode())

# Send a correlated response back (echo style)
await session.publish_to(msg_ctx, b"hi")
```

## Multicast

The Multicast session allows N:N communication on a named channel. Each
message is delivered to all current participants.


Each channel has a moderator that can add or remove participants. Moderation
can be built into your application or delegated to a separate control service
or the SLIM control plane.


Below are examples using the latest Python bindings, along with explanations of
what happens inside the session layer when a participant is added or removed
from the channel.

### Create a Multicast Session

To create a multicast session, you need to configure the session with a topic
name, set yourself as moderator if you want to manage participants, and specify
other options such as retries, timeout, and security settings. Here is an
example:

```python
# Assume local_app is an initialized application instance
session = await local_app.create_session(
    slim_bindings.PySessionConfiguration.Multicast(
        topic=chat_topic,
        max_retries=5,
        timeout=datetime.timedelta(seconds=5),
        mls_enabled=True,
    )
)
```

Parameters:
* `topic` (required, PyName): Channel/Topic name where all the messages are
    delivered.
* `max_retries` (optional, int): Retry attempts for missing Acks.
* `timeout` (optional, timedelta): Wait per attempt for Ack before retry.
* `mls_enabled` (optional, bool): Enable secure group MLS messaging.

If `timeout` is not set the session is best‑effort.

### Sending and Replying in Multicast
In Multicast the session targets a channel: all sends are broadcast to current
participants. Use `publish` to send. `publish_with_destination` cannot be used
in Multicast because the channel name is implicit in the session.

```python
# Broadcast to the channel
await session.publish(b"hello")

# Handle inbound messages
msg_ctx, data = await session.get_message()
print("channel received:", data.decode())

# Reply to every participant
await session.publish(b"hi")
```

### Invite a New Participant

A moderator can invite a new participant to the channel using the `invite`
method after creating the session.

```python
# After creating the session as moderator:
invite_name = split_id(invite)  # Use the participant's ID string
await local_app.set_route(invite_name)
await session.invite(invite_name)
```

Parameter:
* `invite_name` (PyName): Identifier of the participant to add.


When a moderator wants to add a new participant (e.g., an instance of App-C) to
a multicast session, the following steps occur. All the steps are visualized in
the diagram below:


1. **Discovery Phase:** The moderator initiates a discovery request to find a
    running instance of the desired application (App-C). This request is sent to
    the SLIM Node, which forwards it in anycast to one of the App-C instances.
    In the example, the message is forwarded to App-C/1 that replies with its
    full identifier. The SLIM Node relays this reply back to the moderator.

2. **Invitation:** The moderator sends an invite message for the discovered
    instance (App-C/1) to the SLIM Node, which forwards it to App-C/1. Upon
    receiving the invite, App-C/1 creates a new multicast session, subscribes to
    the channel, and replies with its MLS (Messaging Layer Security) key
    package. This reply is routed back to the moderator.

3. **MLS State Update:** The moderator initiates an MLS commit to add App-C/1
    to the secure group. The message is sent using the channel name and so the
    SLIM Node distributes this commit to all current participants (App-B/2 and
    App-A/1), who update their MLS state and acknowledge the commit. The
    moderator collects all acknowledgments. Once all acknowledgments are
    received, the moderator sends an MLS Welcome message to App-C/1. App-C/1
    initializes its MLS state and acknowledges receipt. At the end of this
    process, all participants (including the new one) share a secure group state
    and can exchange encrypted messages on the multicast channel. If MLS is
    disabled, the MLS state update and welcome step are skipped.

```mermaid
sequenceDiagram
    autonumber

    participant Moderator
    participant SLIM Node
    participant App-C/1
    participant App-C/2
    participant App-B/2
    participant App-A/1

    Note over Moderator,App-A/1: Discovery
    Moderator->>SLIM Node: Discover agntcy/ns/App-C
    SLIM Node->>App-C/1: Discover agntcy/ns/App-C
    App-C/1->>SLIM Node: Discover Reply from agntcy/ns/App-C/1
    SLIM Node->>Moderator: Discover Reply from agntcy/ns/App-C/1

    Note over Moderator,App-A/1: Invite
    Moderator->>SLIM Node: Invite agntcy/ns/App-C/1 
    SLIM Node->>App-C/1: Invite agntcy/ns/App-C/1
    App-C/1->>App-C/1: Create new Multicast session
    App-C/1->>SLIM Node: Subscribe to Channel
    App-C/1->>SLIM Node: Invite Reply (MLS key package)
    SLIM Node->>Moderator: Invite Reply (MLS key package)
    Moderator->>Moderator: Update MLS state

    Note over Moderator,App-A/1: MLS State Update
    Moderator->>SLIM Node: MLS commit (Add agntcy/ns/App-C/1) to Channel
    par Handle MLS commit on App-C/1
        SLIM Node->>App-B/2: MLS commit (Add agntcy/ns/App-C/1) to Channel
        App-B/2->>App-B/2: Update MLS state
        App-B/2->>SLIM Node: Ack(MLS Commit)
        SLIM Node->>Moderator: Ack(MLS Commit)
    and Handle MLS commit on App-A/1
        SLIM Node->>App-A/1: MLS commit (Add agntcy/ns/App-C/1) to Channel
        App-A/1->>App-A/1: Update MLS state
        App-A/1->>SLIM Node: Ack(MLS Commit)
        SLIM Node->>Moderator: Ack(MLS Commit)
    end
    Moderator->>SLIM Node: MLS Welcome agntcy/ns/App-C/1
    SLIM Node->>App-C/1: MLS Welcome agntcy/ns/App-C/1
    App-C/1->>App-C/1: Init MLS state
    App-C/1->>SLIM Node: Ack(MLS Welcome)
    SLIM Node->>Moderator: Ack(MLS Welcome)
```

### Remove a Participant


A moderator can remove a participant from the channel using the `remove`
method after creating the session.

```python
# To remove a participant from the session:
remove_name = split_id(participant)  # Use the participant's ID string
await session.remove(remove_name)
```


Parameter:
* `remove_name` (PyName): Identifier of the participant to remove.


When a moderator wants to remove a participant (e.g., App-C/1) from a multicast
session, the following steps occur. All the steps are visualized in the diagram
below:


1. **MLS State Update:** The moderator creates an MLS commit to remove App-C/1
    from the secure group. This commit is sent to the multicast channel and the
    SLIM Node distributes it to all current participants (App-C/1, App-B/2, and
    App-A/1). Each participant updates its MLS state and acknowledges the
    commit. The moderator collects all acknowledgments. In case the MLS is
    disabled, this step is not executed.

2. **Removal:** After the MLS state is updated, the moderator sends a remove
    message to App-C/1. Upon receiving the remove message, App-C/1 unsubscribes
    from the channel, deletes its multicast session, and replies with a
    confirmation. The SLIM Node relays this confirmation back to the moderator.
    At the end of this process, App-C/1 is no longer a member of the multicast
    group and cannot send or receive messages on the channel.

```mermaid
sequenceDiagram
    autonumber

    participant Moderator
    participant SLIM Node
    participant App-C/1
    participant App-B/2
    participant App-A/1

    Note over Moderator,App-A/1: MLS State Update
    Moderator->>SLIM Node: MLS commit (Remove agntcy/ns/App-C/1) to Channel
    par Handle MLS commit on App-C/1
        SLIM Node->>App-C/1: MLS commit (Remove agntcy/ns/App-C/1) to Channel
        App-C/1->>App-C/1: Update MLS state
        App-C/1->>SLIM Node: Ack(MLS Commit)
        SLIM Node->>Moderator: Ack(MLS Commit)
    and Handle MLS commit on App-B/2
        SLIM Node->>App-B/2: MLS commit (Remove agntcy/ns/App-C/1) to Channel
        App-B/2->>App-B/2: Update MLS state
        App-B/2->>SLIM Node: Ack(MLS Commit)
        SLIM Node->>Moderator: Ack(MLS Commit)
    and Handle MLS commit on App-A/1
        SLIM Node->>App-A/1: MLS commit (Remove agntcy/ns/App-C/1) to Channel
        App-A/1->>App-A/1: Update MLS state
        App-A/1->>SLIM Node: Ack(MLS Commit)
        SLIM Node->>Moderator: Ack(MLS Commit)
    end
    
    Note over Moderator,App-A/1: Remove
    Moderator->>SLIM Node: Remove agntcy/ns/App-C/1 
    SLIM Node->>App-C/1: Remove agntcy/ns/App-C/1 
    App-C/1->>SLIM Node: Unsubscribe from Channel
    App-C/1->>App-C/1: Remove Multicast session
    App-C/1->>SLIM Node: Remove Reply
    SLIM Node->>Moderator: Remove Reply
```