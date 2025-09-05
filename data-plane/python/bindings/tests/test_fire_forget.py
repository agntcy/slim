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

    # create new slim object
    sender = await create_slim(sender_name, "secret")

    # Connect to the service and subscribe for the local name
    _ = await sender.connect(
        {"endpoint": "http://127.0.0.1:22345", "tls": {"insecure": True}}
    )

    # set route to receiver
    await sender.set_route(receiver_name)

    receiver_counts = {i: 0 for i in range(10)}

    # run 10 receivers concurrently
    async def run_receiver(i: int):
        # create new receiver object
        receiver = await create_slim(receiver_name, "secret")

        # Connect to the service and subscribe for the local name
        _ = await receiver.connect(
            {"endpoint": "http://127.0.1:22345", "tls": {"insecure": True}}
        )

        async with receiver:
            # wait for a new session
            session_info_rec, _ = await receiver.receive()

            print(f"Receiver {i} received session: {session_info_rec.id}")

            # new session received! listen for the message
            while True:
                info, _ = await receiver.receive(session=session_info_rec.id)

                if (
                    info.destination_name.equal_without_id(receiver_name)
                    and info.payload_type == "hello message"
                    and info.metadata.get("sender") == "hello"
                ):
                    # store the count in dictionary
                    receiver_counts[i] += 1

    # run 10 receivers concurrently
    tasks = []
    for i in range(10):
        t = asyncio.create_task(run_receiver(i))
        tasks.append(t)
        await asyncio.sleep(0.1)

    # create a new session
    session_info = await sender.create_session(
        slim_bindings.PySessionConfiguration.FireAndForget(
            max_retries=5,
            timeout=datetime.timedelta(seconds=5),
            sticky=True,
            mls_enabled=mls_enabled,
        )
    )

    # Wait a moment
    await asyncio.sleep(2)

    payload_type = "hello message"
    metadata = {"sender": "hello"}

    # send a message to the receiver
    for i in range(1000):
        await sender.publish(
            session_info, b"Hello from sender", receiver_name, payload_type, metadata
        )

    # Wait for all receivers to finish
    await asyncio.sleep(1)

    # As we setup a sticky session, all the message should be received by only one
    # receiver. Check that the count is 1000 for one of the receivers
    assert 1000 in receiver_counts.values()

    # Kill all tasks
    for task in tasks:
        task.cancel()
