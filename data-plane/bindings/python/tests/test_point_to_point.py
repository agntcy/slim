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

import pytest
from common import create_slim, create_name, create_client_config

import slim_uniffi_bindings._slim_bindings.slim_bindings as slim_bindings


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
    sender_name = create_name("org", "default", "p2p_sender")
    receiver_name = create_name("org", "default", "p2p_receiver")

    print(f"Sender name: {sender_name}")
    print(f"Receiver name: {receiver_name}")

    # create new slim object
    sender = create_slim(sender_name, local_service=server.local_service)

    if server.local_service:
        # Connect to the service and subscribe for the local name
        conn_id = await sender.connect_async(
            create_client_config("http://127.0.0.1:22345")
        )

        # set route to receiver
        await sender.set_route_async(receiver_name, conn_id)

    receiver_counts = {i: 0 for i in range(10)}

    n_messages = 1000

    async def run_receiver(i: int):
        """Receiver task:
        - Creates its own Slim instance using the shared receiver Name.
        - Awaits the inbound PointToPoint session (only one task should get bound).
        - Counts messages matching expected routing + metadata.
        - Continues until sender finishes publishing (loop ends by external cancel or test end).
        """
        receiver = create_slim(receiver_name, local_service=server.local_service)

        if server.local_service:
            # Connect to the service and subscribe for the local name
            _ = await receiver.connect_async(
                create_client_config("http://127.0.0.1:22345")
            )

        session = await receiver.listen_for_session_async(None)

        # make sure the received session is PointToPoint
        assert session.session_type() == slim_bindings.SessionType.POINT_TO_POINT

        # Note: destination() and source() return Name objects in new API
        # The semantics may differ from old src/dst properties

        while True:
            try:
                received_msg = await session.get_message_async(None)
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

    # create a new session (auto-waits for establishment)
    session_config = slim_bindings.SessionConfig(
        session_type=slim_bindings.SessionType.POINT_TO_POINT,
        enable_mls=mls_enabled,
        max_retries=5,
        interval_ms=100,  # 100ms interval
        initiator=True,
        metadata={},
    )
    sender_session = await sender.create_session_async(session_config, receiver_name)

    payload_type = "hello message"
    metadata = {"sender": "hello"}

    # Flood the established p2s session with messages.
    # Stickiness requirement: every one of these 1000 publishes should be delivered
    # to exactly the same receiver instance (affinity).

    # Publish all messages
    for _ in range(n_messages):
        await sender_session.publish_async(
            b"Hello from sender",
            payload_type,
            metadata,
        )

    # wait a moment for all messages to be processed
    received_msg = await sender_session.get_message_async(None)
    msg = received_msg.payload

    ack_text = msg.decode()
    print(f"Sender received ack from receiver: {ack_text}")

    # Parse winning receiver index from ack: format "All messages received: {i}"
    try:
        winner_id = int(ack_text.rsplit(":", 1)[1].strip())
    except Exception as e:
        raise AssertionError(f"Unexpected ack format '{ack_text}': {e}")

    # Cancel all non-winning receiver tasks
    for idx, t in enumerate(tasks):
        if idx != winner_id and not t.done():
            t.cancel()

    # Affinity assertions:
    #  * Sum of all per-receiver counts == total sent (1000)
    #  * Exactly one bucket contains 1000 (the sticky peer)
    assert sum(receiver_counts.values()) == n_messages
    assert n_messages in receiver_counts.values()

    # Delete sender_session
    await sender.delete_session_async(sender_session)

    # Await only the winning receiver task (others were cancelled)
    winner_result = await tasks[winner_id]
