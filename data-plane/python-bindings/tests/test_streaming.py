# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import time
import datetime
import pytest
import pytest_asyncio
import agp_bindings


@pytest_asyncio.fixture(scope="function")
async def server(request):
    # create new server
    global svc_server
    svc_server = await agp_bindings.create_pyservice("cisco", "default", "server")

    # configure it
    svc_server.configure(
        agp_bindings.GatewayConfig(endpoint=request.param, insecure=True)
    )

    # init tracing
    agp_bindings.init_tracing(log_level="info")

    # run gateway server in background
    ret = await agp_bindings.serve(svc_server)

    # wait for the server to start
    await asyncio.sleep(1)

    # return the server
    yield svc_server


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12365"], indirect=True)
async def test_streaming(server):
    org = "cisco"
    ns = "default"
    agent = "producer"

    broadcast_topic = "broadcast"

    pub_msg = "Hello from producer"

    # create new gateway object
    producer = await agp_bindings.Gateway.new(org, ns, agent)
    producer.configure(
        agp_bindings.GatewayConfig(endpoint="http://127.0.0.1:12365", insecure=True)
    )

    # Connect to the service and subscribe for the local name
    _ = await producer.connect()

    # subscribe to the service, so that it can receive messages from consumer agents
    # asking for retransmissions
    await producer.subscribe(org, ns, agent, producer.id())

    # set route for the producer, so that messages can be sent to consumer that
    # subscribed to the producer topic
    await producer.set_route(org, ns, broadcast_topic)

    # message counr
    count = 10000

    # create 10 consumer agents
    consumers = []
    for i in range(10):

        async def background_task(index):
            name = f"consumer-{index}"

            print(f"Creating consumer {name}...")

            consumer = await agp_bindings.Gateway.new(org, ns, name)
            consumer.configure(
                agp_bindings.GatewayConfig(
                    endpoint="http://127.0.0.1:12365", insecure=True
                )
            )

            # Connect to gateway server
            _ = await consumer.connect()

            # Subscribe as consumer
            await consumer.subscribe(org, ns, name, consumer.id())

            # Subscribe to the producer topic
            await consumer.subscribe(org, ns, broadcast_topic)

            print(f"{name} -> Waiting for new sessions...")
            recv_session, _ = await consumer.receive()

            # new session!
            print(f"{name} -> New session:", recv_session.id)

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
                        f"{name} -> Received: {msg_rcv.decode()}, local count: {local_count}"
                    )

                    # if we reached the count, exit
                    if local_count >= count:
                        print(f"{name} -> Received all messages, exiting...")
                        break
                except Exception as e:
                    print(f"{name} -> Error receiving message: {e}")
                    break

            print(f"{name} -> Exiting...")

        task = asyncio.create_task(background_task(i))
        task.set_name(f"consumer-{i}")
        consumers.append(task)

    # create streaming session with default config
    session_info = await producer.create_streaming_session(
        agp_bindings.PyStreamingConfiguration(
            agp_bindings.PySessionDirection.SENDER,
            topic=None,
            max_retries=5,
            timeout=datetime.timedelta(seconds=5),
        )
    )

    # wait a bit for all the consumers to be ready
    print("Waiting for consumers to be ready...")
    await asyncio.sleep(5)

    # send a message to all consumers
    for i in range(count):
        print(f"{agent} -> Sending message to all consumers...")
        await producer.publish(
            session_info,
            f"{pub_msg} - {i}".encode(),
            org,
            ns,
            broadcast_topic,
        )

    # Wait for the task to complete
    for task in consumers:
        await task
