# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import time
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
@pytest.mark.parametrize("server", ["127.0.0.1:12355"], indirect=True)
async def test_request_reply_base(server):
    org = "cisco"
    ns = "default"
    agent1 = "gateway1"

    # create new gateway object
    gateway1 = await agp_bindings.Gateway.new(org, ns, agent1)
    gateway1.configure(
        agp_bindings.GatewayConfig(endpoint="http://127.0.0.1:12355", insecure=True)
    )

    # Connect to the service and subscribe for the local name
    _ = await gateway1.connect()

    # subscribe to the service
    await gateway1.subscribe(org, ns, agent1)

    # create second local agent
    agent2 = "gateway2"
    gateway2 = await agp_bindings.Gateway.new(org, ns, agent2)
    gateway2.configure(
        agp_bindings.GatewayConfig(endpoint="http://127.0.0.1:12355", insecure=True)
    )

    # Connect to gateway server
    _ = await gateway2.connect()
    await gateway2.subscribe(org, ns, agent2, gateway2.id())

    # set route
    await gateway2.set_route("cisco", "default", agent1)

    # create request/reply session with default config
    session_info = await gateway2.create_rr_session()

    # messages
    pub_msg = str.encode("thisistherequest")
    res_msg = str.encode("thisistheresponse")

    # publish message
    await gateway2.publish(session_info, pub_msg, org, ns, agent1)

    # receive message
    session_info_rec, _ = await gateway1.receive()
    session_info_rec, msg_rcv = await gateway1.receive(session=session_info.id)

    # check if the message is correct
    assert msg_rcv == bytes(pub_msg)

    # make sure the session info is correct
    assert session_info.id == session_info_rec.id

    # reply to gateway 2
    await gateway1.publish_to(session_info_rec, res_msg)

    # wait for message
    _, msg_rcv = await gateway2.receive(session=session_info.id)

    # check if the message is correct
    assert msg_rcv == bytes(res_msg)

    # Try to send another reply to gateway2. This should not reach it
    # because there is no pending request
    await gateway1.publish_to(session_info_rec, res_msg)

    # wait for message on Alice
    try:
        _, msg_rcv = await asyncio.wait_for(
            gateway2.receive(session=session_info.id), timeout=3
        )
    except asyncio.TimeoutError:
        msg_rcv = None

    assert msg_rcv is None

    # let's try now to send a request to gateway1, but this time we will not
    # send a reply

    # publish message
    await gateway2.publish(session_info, pub_msg, org, ns, agent1)

    # Make sure the message is received
    session_info_rec, msg_rcv = await gateway1.receive(session=session_info.id)
    assert msg_rcv == bytes(pub_msg)

    # make sure the session info is correct
    assert session_info.id == session_info_rec.id

    # wait for message gatewaw2. this should raise a timeout error
    try:
        _, msg_rcv = await gateway2.receive(session=session_info.id)
    except Exception:
        # TODO: make sure the message contains the timeout error
        pass


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12356"], indirect=True)
async def test_request_reply(server):
    org = "cisco"
    ns = "default"
    agent1 = "gateway1"

    # create new gateway object
    gateway1 = await agp_bindings.Gateway.new(org, ns, agent1)
    gateway1.configure(
        agp_bindings.GatewayConfig(endpoint="http://127.0.0.1:12356", insecure=True)
    )

    # Connect to the service and subscribe for the local name
    _ = await gateway1.connect()

    # subscribe to the service
    await gateway1.subscribe(org, ns, agent1)

    # create second local agent
    agent2 = "gateway2"
    gateway2 = await agp_bindings.Gateway.new(org, ns, agent2)
    gateway2.configure(
        agp_bindings.GatewayConfig(endpoint="http://127.0.0.1:12356", insecure=True)
    )

    # Connect to gateway server
    _ = await gateway2.connect()
    await gateway2.subscribe(org, ns, agent2, gateway2.id())

    # set route
    await gateway2.set_route("cisco", "default", agent1)

    # create request/reply session with default config
    session_info = await gateway2.create_rr_session()

    # messages
    pub_msg = str.encode("thisistherequest")
    res_msg = str.encode("thisistheresponse")

    # create background task for gateway1
    async def background_task():
        while True:
            try:
                # wait for message from any session
                recv_session, _ = await gateway1.receive()

                # receive message from session
                recv_session, msg_rcv = await gateway1.receive(session=recv_session.id)

                # make sure the message is correct
                assert msg_rcv == bytes(pub_msg)

                # reply to the session
                await gateway1.publish_to(recv_session, res_msg)
            except Exception as e:
                print("Error receiving message on gateway1:", e)
                break

    asyncio.create_task(background_task())

    # send a request and expect a response in gateway2
    session_info, message = await gateway2.request_reply(
        session_info, pub_msg, org, ns, agent1
    )

    # check if the message is correct
    assert message == bytes(res_msg)
