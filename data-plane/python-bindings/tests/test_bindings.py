# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import time
import pytest
import pytest_asyncio
import agp_bindings

# create svcs
svc_server = agp_bindings.PyService("gateway/server")
svc_server.configure(
    agp_bindings.GatewayConfig(endpoint="0.0.0.0:12345", insecure=True)
)


@pytest_asyncio.fixture(scope="module")
async def server():
    # init tracing
    agp_bindings.init_tracing()

    # run gateway server in background
    await agp_bindings.serve(svc_server)

    # wait for the server to start
    await asyncio.sleep(1)


# @pytest.mark.asyncio
# async def test_end_to_end(server):
#     # create 2 clients, Alice and Bob
#     svc_alice = agp_bindings.PyService("gateway/alice")
#     svc_alice.configure(agp_bindings.GatewayConfig(endpoint="http://127.0.0.1:12345"))

#     svc_bob = agp_bindings.PyService("gateway/bob")
#     svc_bob.configure(agp_bindings.GatewayConfig(endpoint="http://127.0.0.1:12345"))

#     # connect to the gateway server
#     await agp_bindings.create_agent(svc_alice, "cisco", "default", "alice", 1234)
#     await agp_bindings.create_agent(svc_bob, "cisco", "default", "bob", 1234)

#     # connect to the service
#     conn_id_alice = await agp_bindings.connect(svc_alice)
#     conn_id_bob = await agp_bindings.connect(svc_bob)

#     # subscribe alice and bob
#     alice_class = agp_bindings.PyAgentClass("cisco", "default", "alice")
#     bob_class = agp_bindings.PyAgentClass("cisco", "default", "bob")
#     await agp_bindings.subscribe(svc_alice, conn_id_alice, alice_class, 1234)
#     await agp_bindings.subscribe(svc_bob, conn_id_bob, bob_class, None)

#     # set routes
#     await agp_bindings.set_route(svc_alice, conn_id_alice, bob_class, None)

#     # create fire and forget session
#     session_id = await agp_bindings.create_session(
#         svc_alice, agp_bindings.PySessionType.FireAndForget
#     )

#     # wait for the routes to be set
#     # TODO remove this sleep
#     await asyncio.sleep(1)

#     # send msg from Alice to Bob
#     msg = [1, 2, 3, 4, 5, 6, 7, 8, 9]
#     await agp_bindings.publish(svc_alice, session_id, 1, msg, bob_class, None)

#     # receive message from Alice
#     session_info, source, msg_rcv = await agp_bindings.receive(svc_bob)

#     # make seure the session id corresponds
#     assert session_info.id == session_id

#     # check if the message is correct
#     assert msg_rcv == bytes(msg)

#     # reply to Alice
#     await agp_bindings.publish(svc_bob, session_info.id, 1, msg_rcv, agent=source)

#     # wait for message
#     _, source, msg_rcv = await agp_bindings.receive(svc_alice)

#     print(msg_rcv)

#     # check if the message is correct
#     assert msg_rcv == bytes(msg)

#     # disconnect alice
#     await agp_bindings.disconnect(svc_alice, conn_id_alice)

#     # disconnect bob
#     await agp_bindings.disconnect(svc_bob, conn_id_bob)


@pytest.mark.asyncio
async def test_gateway_wrapper(server):
    # create new gateway object
    gateway1 = agp_bindings.Gateway("gateway/gateway1")
    gateway1.configure(
        agp_bindings.GatewayConfig(endpoint="http://127.0.0.1:12345", insecure=True)
    )

    org = "cisco"
    ns = "default"
    agent1 = "gateway1"

    # Connect to the gateway server
    local_agent_id1 = await gateway1.create_agent(org, ns, agent1)

    # Connect to the service and subscribe for the local name
    _ = await gateway1.connect()

    # subscribe to the service
    await gateway1.subscribe(org, ns, agent1)

    # create second local agent
    gateway2 = agp_bindings.Gateway("gateway/gateway2")
    gateway2.configure(
        agp_bindings.GatewayConfig(endpoint="http://127.0.0.1:12345", insecure=True)
    )

    agent2 = "gateway2"

    local_agent_id2 = await gateway2.create_agent(org, ns, agent2)

    # Connect to gateway server
    _ = await gateway2.connect()
    await gateway2.subscribe(org, ns, agent2, local_agent_id2)

    # set route
    await gateway2.set_route("cisco", "default", agent1)

    # create session
    session_id = await gateway2.create_session(agp_bindings.PySessionType.FireAndForget)

    # TODO remove this sleep
    await asyncio.sleep(1)

    # publish message
    msg = [1, 2, 3, 4, 5, 6, 7, 8, 9]
    await gateway2.publish(session_id, msg, org, ns, agent1)

    await asyncio.sleep(1)

    # # receive message
    session_info, source, msg_rcv = await gateway1.receive()

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # make sure the session info is correct
    assert session_info.id == session_id

    # reply to Alice
    await gateway1.publish_to(session_info.id, msg_rcv, source)

    # wait for message
    _, source, msg_rcv = await gateway2.receive()

    # check if the message is correct
    assert msg_rcv == bytes(msg)

@pytest.mark.asyncio
async def test_auto_reconnect_after_server_restart():
    # set up a new server
    svc_server = agp_bindings.PyService("gateway/server1")
    svc_server.configure(
        agp_bindings.GatewayConfig(endpoint="0.0.0.0:12346", insecure=True)
    )
    await agp_bindings.serve(svc_server)

    # setup two clients (Alice and Bob)
    svc_alice = agp_bindings.PyService("gateway/alice")
    svc_alice.configure(agp_bindings.GatewayConfig(
        endpoint="http://127.0.0.1:12346"
    ))
    svc_bob = agp_bindings.PyService("gateway/bob")
    svc_bob.configure(agp_bindings.GatewayConfig(
        endpoint="http://127.0.0.1:12346"
    ))
    
    # create agents for Alice and Bob
    await agp_bindings.create_agent(svc_alice, "cisco", "default", "alice", 1234)
    await agp_bindings.create_agent(svc_bob, "cisco", "default", "bob", 1234)
    
    # connect clients and subscribe for messages
    conn_id_alice = await agp_bindings.connect(svc_alice)
    conn_id_bob = await agp_bindings.connect(svc_bob)
    
    alice_class = agp_bindings.PyAgentClass("cisco", "default", "alice")
    bob_class = agp_bindings.PyAgentClass("cisco", "default", "bob")
    await agp_bindings.subscribe(svc_alice, conn_id_alice, alice_class, 1234)
    await agp_bindings.subscribe(svc_bob, conn_id_bob, bob_class, 1234)
    
    # set routing from Alice to Bob
    await agp_bindings.set_route(svc_alice, conn_id_alice, bob_class, None)

    # create fire and forget session
    session_id = await agp_bindings.create_session(
        svc_alice, agp_bindings.PySessionType.FireAndForget
    )
    
    # verify baseline message exchange before the simulated server restart
    baseline_msg = [1, 2, 3]
    await agp_bindings.publish(svc_alice, session_id, 1, baseline_msg, bob_class, None)
    
    _, src, received = await agp_bindings.receive(svc_bob)
    assert received == bytes(baseline_msg)
    
    # --- simulate gateway server restart ---
    # stop the current server instance
    await agp_bindings.stop(svc_server)
    await asyncio.sleep(3)  # allow time for the server to fully shut down
    await agp_bindings.serve(svc_server)
    await asyncio.sleep(2)  # allow time for automatic reconnection
    
    # test that the message exchange resumes normally after the simulated restart
    test_msg = [4, 5, 6]
    await agp_bindings.publish(svc_alice, session_id, 1, test_msg, bob_class, None)
    _, src, received = await agp_bindings.receive(svc_bob)
    assert received == bytes(test_msg)
    
    # clean up
    await agp_bindings.disconnect(svc_alice, conn_id_alice)
    await agp_bindings.disconnect(svc_bob, conn_id_bob)
