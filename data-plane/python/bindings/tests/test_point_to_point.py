# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime

import pytest
from common import create_slim

import slim_bindings


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:22345"], indirect=True)
@pytest.mark.parametrize("mls_enabled", [True, False])
async def test_sticky_session(server, mls_enabled):
    sender_name = slim_bindings.PyName("org", "default", "sender")
    receiver_name = slim_bindings.PyName("org", "default", "receiver")

    print(f"Sender name: {sender_name}")
    print(f"Receiver name: {receiver_name}")

    # create new slim object
    sender = await create_slim(sender_name, "secret")

    # Connect to the service and subscribe for the local name
    _ = await sender.connect(
        {"endpoint": "http://127.0.0.1:22345", "tls": {"insecure": True}}
    )

    # set route to receiver
    await sender.set_route(receiver_name)

    receiver_counts = {i: 0 for i in range(10)}

    async def run_receiver(i: int):
        receiver = await create_slim(receiver_name, "secret")
        # Connect to the service and subscribe for the local name
        _ = await receiver.connect(
            {"endpoint": "http://127.0.0.1:22345", "tls": {"insecure": True}}
        )

        session = await receiver.listen_for_session()

        # make sure the received session is unicast
        assert session.session_type == slim_bindings.PySessionType.UNICAST

        # Make sure the dst of the session is the receiver name
        assert session.dst == receiver_name

        # Make sure the src of the session is the sender
        assert session.src == sender.local_name


        while True:
            try:
                _ctx, _ = await session.get_message()
            except Exception as e:
                print(f"Receiver {i} error: {e}")
                break

            if (
                _ctx.destination_name.equal_without_id(receiver_name)
                and _ctx.payload_type == "hello message"
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
        slim_bindings.PySessionConfiguration.Unicast(
            max_retries=5,
            timeout=datetime.timedelta(seconds=5),
            mls_enabled=mls_enabled,
        )
    )

    # Wait a moment
    await asyncio.sleep(2)

    payload_type = "hello message"
    metadata = {"sender": "hello"}

    # send a message to the receiver
    for _ in range(1000):
        await sender_session.publish(
            b"Hello from sender",
            receiver_name,
            payload_type=payload_type,
            metadata=metadata,
        )

    # Wait for all receivers to finish
    await asyncio.sleep(1)

    # As we setup a sticky session, all the message should be received by only one
    # receiver. Check that the count is 1000 for one of the receivers
    # Expect all sent messages go to exactly one receiver due to stickiness
    assert sum(receiver_counts.values()) == 1000
    assert 1000 in receiver_counts.values()

    # Kill all tasks
    for t in tasks:
        t.cancel()
