# SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import time
import pytest
import pytest_asyncio
import agp_bindings

# create svcs
svc_server = agp_bindings.PyService("gateway/server")


@pytest_asyncio.fixture(scope="module")
async def server():
    # init tracing
    agp_bindings.init_tracing()

    # run gateway server in background
    await agp_bindings.serve(svc_server, "0.0.0.0:12345", insecure=True)

    # wait for the server to start
    await asyncio.sleep(1)


@pytest.mark.asyncio
async def test_end_to_end(server):
    # create 2 clients, Alice and Bob
    svc_alice = agp_bindings.PyService("gateway/alice")
    svc_bob = agp_bindings.PyService("gateway/bob")

    # connect to the gateway server
    await agp_bindings.create_agent(svc_alice, "cisco", "default", "alice", 1234)
    await agp_bindings.create_agent(svc_bob, "cisco", "default", "bob", 1234)

    # connect to the service
    conn_id_alice = await agp_bindings.connect(svc_alice, "http://127.0.0.1:12345")
    conn_id_bob = await agp_bindings.connect(svc_bob, "http://127.0.0.1:12345")

    # subscribe alice and bob
    alice_class = agp_bindings.PyAgentClass("cisco", "default", "alice")
    bob_class = agp_bindings.PyAgentClass("cisco", "default", "bob")
    await agp_bindings.subscribe(svc_alice, conn_id_alice, alice_class, 1234)
    await agp_bindings.subscribe(svc_bob, conn_id_bob, bob_class, 1234)

    # set routes
    await agp_bindings.set_route(svc_alice, conn_id_alice, bob_class, None)

    # wait for the routes to be set
    # TODO remove this sleep
    await asyncio.sleep(1)

    # send msg from Alice to Bob
    msg = [1, 2, 3, 4, 5, 6, 7, 8, 9]
    dest = agp_bindings.PyAgentClass("cisco", "default", "bob")
    await agp_bindings.publish(svc_alice, 1, msg, dest, None)

    # receive message from Alice
    source, msg_rcv = await agp_bindings.receive(svc_bob)

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # reply to Alice
    await agp_bindings.publish(svc_bob, 1, msg_rcv, agent=source)

    # wait for message
    source, msg_rcv = await agp_bindings.receive(svc_alice)

    print(msg_rcv)

    # check if the message is correct
    assert msg_rcv == bytes(msg)


@pytest.mark.asyncio
async def test_gateway_wrapper(server):
    # create new gateway object
    gateway1 = agp_bindings.Gateway("gateway/gateway1")

    org = "cisco"
    ns = "default"
    agent1 = "gateway1"

    # create local agent 1
    await gateway1.create_agent(org, ns, agent1)

    # Connect to the gateway server
    local_agent_id1 = await gateway1.create_agent(org, ns, agent1)

    # Connect to the service and subscribe for the local name
    _ = await gateway1.connect("http://127.0.0.1:12345", insecure=True)

    # disconnect and reconnect
    await gateway1.disconnect()
    time.sleep(1)

    _ = await gateway1.connect("http://127.0.0.1:12345", insecure=True)

    await gateway1.subscribe(org, ns, agent1, local_agent_id1)

    # create second local agent
    gateway2 = agp_bindings.Gateway("gateway/gateway2")

    agent2 = "gateway2"

    local_agent_id2 = await gateway2.create_agent(org, ns, agent2)

    # Connect to gateway server
    _ = await gateway2.connect("http://127.0.0.1:12345", insecure=True)
    await gateway2.subscribe(org, ns, agent2, local_agent_id2)

    # set route
    await gateway2.set_route("cisco", "default", agent1)

    # wait for the routes to be set
    # TODO remove this sleep
    await asyncio.sleep(1)

    # publish message
    msg = [1, 2, 3, 4, 5, 6, 7, 8, 9]
    await gateway2.publish(msg, org, ns, agent1)

    # receive message
    source, msg_rcv = await gateway1.receive()

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # reply to Alice
    await gateway1.publish_to(msg_rcv, source)

    # wait for message
    source, msg_rcv = await gateway2.receive()

    # check if the message is correct
    assert msg_rcv == bytes(msg)
