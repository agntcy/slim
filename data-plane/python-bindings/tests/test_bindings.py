# SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import time
import pytest
import pytest_asyncio
import gateway_bindings

# create svcs
svc_server = gateway_bindings.PyService("gateway/server")


@pytest_asyncio.fixture(scope="module")
async def server():
    # init tracing
    gateway_bindings.init_tracing()

    # run gateway server in background
    await gateway_bindings.serve(svc_server, "0.0.0.0:12345", insecure=True)

    # wait for the server to start
    await asyncio.sleep(1)


@pytest.mark.asyncio
async def test_end_to_end(server):
    # create 2 clients, Alice and Bob
    svc_alice = gateway_bindings.PyService("gateway/alice")
    svc_bob = gateway_bindings.PyService("gateway/bob")

    # connect to the gateway server
    await gateway_bindings.create_agent(svc_alice, "cisco", "default", "alice", 1234)
    await gateway_bindings.create_agent(svc_bob, "cisco", "default", "bob", 1234)

    # connect to the service
    conn_id_alice = await gateway_bindings.connect(svc_alice, "http://127.0.0.1:12345")
    conn_id_bob = await gateway_bindings.connect(svc_bob, "http://127.0.0.1:12345")

    # subscribe alice and bob
    alice_class = gateway_bindings.PyAgentClass("cisco", "default", "alice")
    bob_class = gateway_bindings.PyAgentClass("cisco", "default", "bob")
    await gateway_bindings.subscribe(svc_alice, conn_id_alice, alice_class, 1234)
    await gateway_bindings.subscribe(svc_bob, conn_id_bob, bob_class, 1234)

    # set routes
    await gateway_bindings.set_route(svc_alice, conn_id_alice, bob_class, None)

    # send msg from Alice to Bob
    msg = [1, 2, 3, 4, 5, 6, 7, 8, 9]
    dest = gateway_bindings.PyAgentClass("cisco", "default", "bob")
    await gateway_bindings.publish(svc_alice, 1, msg, dest, None)

    # receive message from Alice
    source, msg_rcv = await gateway_bindings.receive(svc_bob)

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # reply to Alice
    await gateway_bindings.publish(svc_bob, 1, msg_rcv, agent=source)

    # wait for message
    source, msg_rcv = await gateway_bindings.receive(svc_alice)

    print(msg_rcv)

    # check if the message is correct
    assert msg_rcv == bytes(msg)


@pytest.mark.asyncio
async def test_gateway_wrapper(server):
    # create new gateway object
    gateway1 = gateway_bindings.Gateway("gateway/gateway1")

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
    gateway2 = gateway_bindings.Gateway("gateway/gateway2")

    agent2 = "gateway2"

    local_agent_id2 = await gateway2.create_agent(org, ns, agent2)

    # Connect to gateway server
    _ = await gateway2.connect("http://127.0.0.1:12345", insecure=True)
    await gateway2.subscribe(org, ns, agent2, local_agent_id2)

    # set route
    await gateway2.set_route("cisco", "default", agent1)

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