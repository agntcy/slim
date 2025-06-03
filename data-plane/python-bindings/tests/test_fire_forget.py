# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime

import pytest

import slim_bindings


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:22345"], indirect=True)
async def test_sticky_session(server):
    org = "org"
    ns = "default"
    sender = "sender"

    # create new slim object
    sender = await slim_bindings.Slim.new(org, ns, sender)

    # Connect to the service and subscribe for the local name
    _ = await sender.connect(
        {"endpoint": "http://127.0.0.1:22345", "tls": {"insecure": True}}
    )

    # set route to receiver
    await sender.set_route(org, ns, "receiver")

    receiver_counts = {i: 0 for i in range(10)}

    # run 10 receivers concurrently
    async def run_receiver(i: int):
        # create new receiver object
        receiver = await slim_bindings.Slim.new(org, ns, "receiver")

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
                _, _ = await receiver.receive(session=session_info_rec.id)

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
        )
    )

    # Wait a moment
    await asyncio.sleep(2)

    # send a message to the receiver
    for i in range(1000):
        await sender.publish(
            session_info,
            b"Hello from sender",
            org,
            ns,
            "receiver",
        )

    # Wait for all receivers to finish
    await asyncio.sleep(1)

    # As we setup a sticky session, all the message should be received by only one
    # receiver. Check that the count is 1000 for one of the receivers
    assert 1000 in receiver_counts.values()

    # Kill all tasks
    for task in tasks:
        task.cancel()
