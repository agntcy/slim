# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""
Point-to-point sticky session integration test.

Scenario:
  - One logical sender creates a PointToPoint session and sends 1000 messages
    to a shared logical receiver identity.
  - Ten receiver instances (same PyName) concurrently listen for an
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

Notes:
  The test uses simple sleeps for propagation; production-grade suites
  might replace those with explicit readiness signaling.
"""

import asyncio
import datetime

import pytest
from common import create_slim

import slim_bindings


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "server",
    [
        "127.0.0.1:22345",  # local service
        None,  # global service
    ],
    indirect=True,
)
@pytest.mark.parametrize("mls_enabled", [False])
async def test_sticky_session(server, mls_enabled):
    """Ensure all messages in a PointToPoint session are delivered to a single receiver instance.

    Args:
        server: Pytest fixture starting the Slim server on a dedicated port.
        mls_enabled (bool): Whether to enable MLS for the created session.

    Flow:
        1. Spawn 10 receiver tasks (same logical PyName).
        2. Sender establishes PointToPoint session.
        3. Sender publishes 1000 messages with consistent metadata + payload_type.
        4. Each receiver tallies only messages addressed to the logical receiver name.
        5. Assert affinity: exactly one receiver processed all messages.

    Expectation:
        Sticky routing pins all messages to the first receiver that accepted the session.
    """
    sender_name = slim_bindings.PyName("org", "default", "p2p_sender")
    receiver_name = slim_bindings.PyName("org", "default", "p2p_receiver")

    print(f"Sender name: {sender_name}")
    print(f"Receiver name: {receiver_name}")

    # create new slim object
    sender = await create_slim(
        sender_name, "secret", local_service=server.local_service
    )

    if server.local_service:
        # Connect to the service and subscribe for the local name
        _ = await sender.connect(
            {"endpoint": "http://127.0.0.1:22345", "tls": {"insecure": True}}
        )

        # set route to receiver
        await sender.set_route(receiver_name)

    receiver_counts = {i: 0 for i in range(10)}

    async def run_receiver(i: int):
        """Receiver task:
        - Creates its own Slim instance using the shared receiver PyName.
        - Awaits the inbound PointToPoint session (only one task should get bound).
        - Counts messages matching expected routing + metadata.
        - Continues until sender finishes publishing (loop ends by external cancel or test end).
        """
        receiver = await create_slim(
            receiver_name, "secret", local_service=server.local_service
        )

        if server.local_service:
            # Connect to the service and subscribe for the local name
            _ = await receiver.connect(
                {"endpoint": "http://127.0.0.1:22345", "tls": {"insecure": True}}
            )

        session = await receiver.listen_for_session()

        # make sure the received session is PointToPoint
        assert session.session_type == slim_bindings.PySessionType.PointToPoint

        # Make sure the dst of the session is the receiver name
        assert session.dst == sender.local_name

        # Make sure the src of the session is the sender
        assert session.src == receiver.local_name

        while True:
            try:
                _ctx, _ = await session.get_message()
            except Exception as e:
                print(f"Receiver {i} error: {e}")
                break

            if (
                _ctx.payload_type == "hello message"
                and _ctx.metadata.get("sender") == "hello"
            ):
                # store the count in dictionary
                receiver_counts[i] += 1

    tasks = []
    for i in range(10):
        t = asyncio.create_task(run_receiver(i))
        tasks.append(t)
        await asyncio.sleep(0.1)

    # create a new session
    sender_session = await sender.create_session(
        slim_bindings.PySessionConfiguration.PointToPoint(
            peer_name=receiver_name,
            max_retries=5,
            timeout=datetime.timedelta(seconds=5),
            mls_enabled=mls_enabled,
        ),
    )

    # Wait a moment
    await asyncio.sleep(2)

    payload_type = "hello message"
    metadata = {"sender": "hello"}

    # Flood the established p2s session with messages.
    # Stickiness requirement: every one of these 1000 publishes should be delivered
    # to exactly the same receiver instance (affinity).
    for _ in range(1000):
        await sender_session.publish(
            b"Hello from sender",
            payload_type=payload_type,
            metadata=metadata,
        )

    # Wait for all receivers to finish
    await asyncio.sleep(1)

    # Affinity assertions:
    #  * Sum of all per-receiver counts == total sent (1000)
    #  * Exactly one bucket contains 1000 (the sticky peer)
    assert sum(receiver_counts.values()) == 1000
    assert 1000 in receiver_counts.values()

    # Delete sender_session
    await sender.delete_session(sender_session)

    await asyncio.sleep(5)

    # Kill all tasks
    for t in tasks:
        t.cancel()
