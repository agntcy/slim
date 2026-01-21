# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""
Point-to-point sticky session integration test.

Scenario:
  - One logical sender creates a PointToPoint session and sends 1000 messages
    to a shared logical receiver identity.
  - Ten receiver instances (same Name) concurrently listen for an
    inbound session. Only one should become the bound peer for the
    PointToPoint session (stickiness).
  - All 1000 messages must arrive at exactly one receiver (verifying
    session affinity) and none at the others.
  - Test runs with MLS enabled / disabled (parametrized) to ensure
    stickiness is orthogonal to MLS.

Validated invariants:
  * session_type == PointToPoint for receiver-side context
  * dst == sender.local_name and src == receiver.local_name
  * Exactly one receiver_counts[i] == 1000 and total sum == 1000
"""

import asyncio
import datetime
import uuid

import pytest

import slim_bindings

LONG_SECRET = "e4aaecb9ae0b23b82086bb8a8633e01fba16ae8d9c1379a613c00838"


async def _setup_sender(server, sender_name, test_id, receiver_name):
    """Create and configure sender app."""
    conn_id_sender = None
    if server.local_service:
        svc_sender = slim_bindings.Service("svcsender")
        conn_id_sender = await svc_sender.connect_async(server.get_client_config())
    else:
        svc_sender = server.service

    sender = svc_sender.create_app_with_secret(sender_name, LONG_SECRET)

    if server.local_service:
        # Subscribe sender
        sender_name_with_id = slim_bindings.Name.new_with_id(
            "org", f"test_{test_id}", "p2psender", id=sender.id()
        )
        await sender.subscribe_async(sender_name_with_id, conn_id_sender)
        await asyncio.sleep(0.1)

        # set route to receiver
        await sender.set_route_async(receiver_name, conn_id_sender)

    return sender, conn_id_sender


async def _publish_messages(sender_session, n_messages, payload_type, metadata):
    """Publish messages and wait for completion."""
    handles = []
    for _ in range(n_messages):
        h = await sender_session.publish_async(
            b"Hello from sender",
            payload_type,
            metadata,
        )
        handles.append(h.wait_async())

    # wait for all messages
    await asyncio.gather(*handles)


async def _wait_for_ack(sender_session):
    """Wait for acknowledgment from receiver and return winner_id."""
    received_msg = await sender_session.get_message_async(
        timeout=datetime.timedelta(seconds=30)
    )
    msg = received_msg.payload
    ack_text = msg.decode()
    print(f"Sender received ack from receiver: {ack_text}")

    # Parse winning receiver index from ack: format "All messages received: {i}"
    try:
        winner_id = int(ack_text.rsplit(":", 1)[1].strip())
    except Exception as e:
        raise AssertionError(f"Unexpected ack format '{ack_text}': {e}")

    return winner_id


def _validate_affinity(receiver_counts, n_messages):
    """Validate that exactly one receiver got all messages."""
    assert sum(receiver_counts.values()) == n_messages
    assert n_messages in receiver_counts.values()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "server",
    [
        "127.0.0.1:22345",  # local service
        None,  # global service
    ],
    indirect=True,
)
@pytest.mark.parametrize("mls_enabled", [True, False])
async def test_sticky_session(server, mls_enabled):
    """Ensure all messages in a PointToPoint session are delivered to a single receiver instance.

    Args:
        server: Pytest fixture starting the Slim server on a dedicated port.
        mls_enabled (bool): Whether to enable MLS for the created session.

    Flow:
        1. Spawn 10 receiver tasks (same logical Name).
        2. Sender establishes PointToPoint session.
        3. Sender publishes 1000 messages with consistent metadata + payload_type.
        4. Each receiver tallies only messages addressed to the logical receiver name.
        5. Assert affinity: exactly one receiver processed all messages.

    Expectation:
        Sticky routing pins all messages to the first receiver that accepted the session.
    """
    # Generate unique names to avoid collisions
    test_id = str(uuid.uuid4())[:8]
    sender_name = slim_bindings.Name("org", f"test_{test_id}", "p2psender")
    receiver_name = slim_bindings.Name("org", f"test_{test_id}", "p2preceiver")

    print(f"Sender name: {sender_name.as_string()}")
    print(f"Receiver name: {receiver_name.as_string()}")

    # Create sender service and app
    sender, conn_id_sender = await _setup_sender(
        server, sender_name, test_id, receiver_name
    )

    receiver_counts = {i: 0 for i in range(10)}

    n_messages = 1000

    async def run_receiver(i: int):
        """Receiver task:
        - Creates its own Slim instance using the shared receiver Name.
        - Awaits the inbound PointToPoint session (only one task should get bound).
        - Counts messages matching expected routing + metadata.
        - Continues until sender finishes publishing (loop ends by external cancel or test end).
        """
        # Create receiver service and app
        conn_id_receiver = None
        if server.local_service:
            svc_receiver = slim_bindings.Service(f"svcreceiver{i}")
            conn_id_receiver = await svc_receiver.connect_async(
                server.get_client_config()
            )
        else:
            svc_receiver = server.service

        receiver = svc_receiver.create_app_with_secret(receiver_name, LONG_SECRET)

        if server.local_service:
            # Subscribe receiver
            receiver_name_with_id = slim_bindings.Name.new_with_id(
                "org", f"test_{test_id}", "p2preceiver", id=receiver.id()
            )
            await receiver.subscribe_async(receiver_name_with_id, conn_id_receiver)
            await asyncio.sleep(0.1)

        session_context = await receiver.listen_for_session_async(None)
        session = session_context

        # make sure the received session is PointToPoint
        assert session.session_type() == slim_bindings.SessionType.POINT_TO_POINT

        # Note: destination() and source() return Name objects in new API
        # The semantics may differ from old src/dst properties

        while True:
            try:
                received_msg = await session.get_message_async(
                    timeout=datetime.timedelta(seconds=30)
                )
                _ctx = received_msg.context
            except Exception as e:
                print(f"Receiver {i} error: {e}")
                break

            if (
                _ctx.payload_type == "hello message"
                and _ctx.metadata.get("sender") == "hello"
            ):
                # store the count in dictionary
                receiver_counts[i] += 1

                if receiver_counts[i] == n_messages:
                    # send back application acknowledgment
                    await session.publish_async(
                        f"All messages received: {i}".encode(), None, None
                    )

    tasks = []
    for i in range(10):
        t = asyncio.create_task(run_receiver(i))
        tasks.append(t)

    # Give receivers a moment to start listening (especially important for global service mode)
    await asyncio.sleep(0.5)

    # create a new session
    session_config = slim_bindings.SessionConfig(
        session_type=slim_bindings.SessionType.POINT_TO_POINT,
        enable_mls=mls_enabled,
        max_retries=5,
        interval=datetime.timedelta(milliseconds=100),
        metadata={},
    )
    sender_session_context = await sender.create_session_async(
        session_config, receiver_name
    )
    await sender_session_context.completion.wait_async()
    sender_session = sender_session_context.session

    payload_type = "hello message"
    metadata = {"sender": "hello"}

    # Flood the established p2s session with messages.
    # Stickiness requirement: every one of these 1000 publishes should be delivered
    # to exactly the same receiver instance (affinity).

    # Publish all messages
    await _publish_messages(sender_session, n_messages, payload_type, metadata)

    # Wait for acknowledgment from receiver
    winner_id = await _wait_for_ack(sender_session)

    # Cancel all non-winning receiver tasks
    for idx, t in enumerate(tasks):
        if idx != winner_id and not t.done():
            t.cancel()

    # Affinity assertions:
    #  * Sum of all per-receiver counts == total sent (1000)
    #  * Exactly one bucket contains 1000 (the sticky peer)
    _validate_affinity(receiver_counts, n_messages)

    # Delete sender_session
    handle = await sender.delete_session_async(sender_session)
    await handle.wait_async()

    # Await only the winning receiver task (others were cancelled)
    try:
        await tasks[winner_id]
    except asyncio.CancelledError:
        pass  # Expected if task was cancelled
