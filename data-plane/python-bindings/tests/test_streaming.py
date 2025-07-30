# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime

import pytest
from common import create_slim

import slim_bindings


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12365"], indirect=True)
async def test_streaming(server):
    org = "cisco"
    ns = "default"
    app_name = "producer"

    broadcast_topic = "broadcast"

    pub_msg = "Hello from producer"

    name = slim_bindings.PyName(org, ns, app_name)
    channel_name = slim_bindings.PyName(org, ns, broadcast_topic)

    # create new SLIM object
    producer = await create_slim(name, "secret")

    # Connect to the service and subscribe for the local name
    _ = await producer.connect(
        {"endpoint": "http://127.0.0.1:12365", "tls": {"insecure": True}}
    )

    # set route for the producer, so that messages can be sent to consumer that
    # subscribed to the producer topic
    await producer.set_route(channel_name)

    # message count
    count = 10000

    # consumer count
    consumers_count = 10
    consumers = []

    # count array
    # each consumer will have its own count
    counts = []

    # define the background task
    async def background_task(index):
        consumer_name = f"consumer-{index}"

        name = slim_bindings.PyName(org, ns, consumer_name)

        print(f"Creating consumer {name}...")

        consumer = await create_slim(name, "secret")

        # Connect to SLIM server
        _ = await consumer.connect(
            {"endpoint": "http://127.0.0.1:12365", "tls": {"insecure": True}}
        )

        # Subscribe to the producer topic
        await consumer.subscribe(channel_name)

        async with consumer:
            print(f"{consumer_name} -> Waiting for new sessions...")
            recv_session, _ = await consumer.receive()

            # new session!
            print(f"{consumer_name} -> New session:", recv_session.id)

            local_count = 0

            while True:
                try:
                    # receive message from session
                    recv_session, msg_rcv = await consumer.receive(
                        session=recv_session.id
                    )

                    # increase the count
                    local_count += 1

                    # make sure the message is correct
                    assert msg_rcv.startswith(bytes(pub_msg.encode()))

                    # print the message
                    print(
                        f"{consumer_name} -> Received: {msg_rcv.decode()}, local count: {local_count}"
                    )

                    # increment the count
                    counts[index] += 1

                    # if we reached the count, exit
                    if local_count >= count:
                        print(f"{consumer_name} -> Received all messages, exiting...")
                        break
                except Exception as e:
                    print(f"{consumer_name} -> Error receiving message: {e}")
                    break

            print(f"{consumer_name} -> Exiting...")

    # start consumers in background
    for i in range(consumers_count):
        # Init count array
        counts.append(0)

        task = asyncio.create_task(background_task(i))
        task.set_name(f"consumer-{i}")
        consumers.append(task)

    # create streaming session with default config
    session_info = await producer.create_session(
        slim_bindings.PySessionConfiguration.Streaming(
            slim_bindings.PySessionDirection.SENDER,
            moderator=False,
            topic=channel_name,
            max_retries=5,
            timeout=datetime.timedelta(seconds=5),
        )
    )

    # wait a bit for all the consumers to be ready
    print("Waiting for consumers to be ready...")
    await asyncio.sleep(5)

    # send a message to all consumers
    for i in range(count):
        print(f"{app_name} -> Sending message to all consumers...")
        await producer.publish(
            session_info,
            f"{pub_msg} - {i}".encode(),
            channel_name,
        )

    # Wait for the task to complete
    for task in consumers:
        await task

    # make sure the counts are correct
    for i in range(consumers_count):
        assert counts[i] == count, (
            f"Consumer {i} received {counts[i]} messages, expected {count}"
        )
