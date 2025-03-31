# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import time
import pytest
import pytest_asyncio
import agp_bindings

ENDPOINT = "127.0.0.1:12346"

# create svcs
svc_server = agp_bindings.PyService("gateway/server")
svc_server.configure(agp_bindings.GatewayConfig(endpoint=ENDPOINT, insecure=True))


@pytest_asyncio.fixture(scope="module")
async def server():
    # init tracing
    agp_bindings.init_tracing()

    # run gateway server in background
    await agp_bindings.serve(svc_server)

    # wait for the server to start
    await asyncio.sleep(1)


@pytest.mark.asyncio
async def test_request_reply(server):
    # create new gateway object
    gateway1 = agp_bindings.Gateway("gateway/gateway1")
    gateway1.configure(
        agp_bindings.GatewayConfig(endpoint=f"http://{ENDPOINT}", insecure=True)
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
        agp_bindings.GatewayConfig(endpoint=f"http://{ENDPOINT}", insecure=True)
    )

    agent2 = "gateway2"

    local_agent_id2 = await gateway2.create_agent(org, ns, agent2)

    # Connect to gateway server
    _ = await gateway2.connect()
    await gateway2.subscribe(org, ns, agent2, local_agent_id2)

    # set route
    await gateway2.set_route("cisco", "default", agent1)

    # create request/reply session with default config
    session_info = await gateway2.create_rr_session()

    # TODO remove this sleep
    await asyncio.sleep(1)

    # messages
    pub_msg = str.encode("thisistherequest")
    res_msg = str.encode("thisistheresponse")

    # publish message
    await gateway2.publish(session_info, pub_msg, org, ns, agent1)

    ## receive message
    session_info_rec, msg_rcv = await gateway1.receive()

    # check if the message is correct
    assert msg_rcv == bytes(pub_msg)

    # make sure the session info is correct
    assert session_info.id == session_info_rec.id

    # reply to gateway 2
    await gateway1.publish_to(session_info_rec, res_msg)

    # wait for message
    _, msg_rcv = await gateway2.receive()

    # check if the message is correct
    assert msg_rcv == bytes(res_msg)

    # Try to send another reply to gateway2. This should not reach it
    # because there is no pending request
    await gateway1.publish_to(session_info_rec, res_msg)

    # wait for message on Alice
    try:
        _, msg_rcv = await asyncio.wait_for(gateway2.receive(), timeout=3)
    except asyncio.TimeoutError:
        msg_rcv = None

    assert msg_rcv is None

    # let's try now to send a request to gateway1, but this time we will not
    # send a reply

    # publish message
    await gateway2.publish(session_info, pub_msg, org, ns, agent1)

    # Make sure the message is received
    session_info_rec, msg_rcv = await gateway1.receive()
    assert msg_rcv == bytes(pub_msg)

    # make sure the session info is correct
    assert session_info.id == session_info_rec.id

    # wait for message gatewaw2. this should raise a timeout error
    try:
        _, msg_rcv = await gateway2.receive()
    except Exception:
        # make sure the message contains the timeout error
        pass
