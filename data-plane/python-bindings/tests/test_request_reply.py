# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio

import pytest

import slim_bindings


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12355"], indirect=True)
async def test_request_reply_base(server):
    org = "cisco"
    ns = "default"
    agent1 = "slim1"

    # create new slim object
    slim1 = await slim_bindings.Slim.new(org, ns, agent1)

    # Connect to the service and subscribe for the local name
    _ = await slim1.connect(
        {"endpoint": "http://127.0.0.1:12355", "tls": {"insecure": True}}
    )

    # # subscribe to the service
    # await slim1.subscribe(org, ns, agent1)

    # create second local agent
    agent2 = "slim2"
    slim2 = await slim_bindings.Slim.new(org, ns, agent2)

    # Connect to SLIM server
    _ = await slim2.connect(
        {"endpoint": "http://127.0.0.1:12355", "tls": {"insecure": True}}
    )

    # set route
    await slim2.set_route("cisco", "default", agent1)

    # create request/reply session with default config
    session_info = await slim2.create_session(
        slim_bindings.PySessionConfiguration.RequestResponse(),
    )

    # messages
    pub_msg = str.encode("thisistherequest")
    res_msg = str.encode("thisistheresponse")

    async with slim1, slim2:
        # publish message
        await slim2.publish(session_info, pub_msg, org, ns, agent1)

        # receive message
        session_info_rec, _ = await slim1.receive()
        session_info_rec, msg_rcv = await slim1.receive(session=session_info.id)

        # check if the message is correct
        assert msg_rcv == bytes(pub_msg)

        # make sure the session info is correct
        assert session_info.id == session_info_rec.id

        # reply to slim 2
        await slim1.publish_to(session_info_rec, res_msg)

        # wait for message
        _, msg_rcv = await slim2.receive(session=session_info.id)

        # check if the message is correct
        assert msg_rcv == bytes(res_msg)

        # Try to send another reply to slim2. This should not reach it
        # because there is no pending request
        await slim1.publish_to(session_info_rec, res_msg)

        # wait for message on Alice
        try:
            _, msg_rcv = await asyncio.wait_for(
                slim2.receive(session=session_info.id), timeout=3
            )
        except asyncio.TimeoutError:
            msg_rcv = None

        assert msg_rcv is None

        # let's try now to send a request to slim1, but this time we will not
        # send a reply

        # publish message
        await slim2.publish(session_info, pub_msg, org, ns, agent1)

        # Make sure the message is received
        session_info_rec, msg_rcv = await slim1.receive(session=session_info.id)
        assert msg_rcv == bytes(pub_msg)

        # make sure the session info is correct
        assert session_info.id == session_info_rec.id

        # wait for message gatewaw2. this should raise a timeout error
        try:
            _, msg_rcv = await slim2.receive(session=session_info.id)
        except Exception:
            # TODO: make sure the message contains the timeout error
            pass


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12356"], indirect=True)
async def test_request_reply(server):
    org = "cisco"
    ns = "default"
    agent1 = "slim1"

    # create new slim object
    slim1 = await slim_bindings.Slim.new(org, ns, agent1)

    # Connect to the service and subscribe for the local name
    _ = await slim1.connect(
        {"endpoint": "http://127.0.0.1:12356", "tls": {"insecure": True}}
    )

    # create second local agent
    agent2 = "slim2"
    slim2 = await slim_bindings.Slim.new(org, ns, agent2)

    # Connect to SLIM server
    _ = await slim2.connect(
        {"endpoint": "http://127.0.0.1:12356", "tls": {"insecure": True}}
    )

    # set route
    await slim2.set_route("cisco", "default", agent1)

    # create request/reply session with default config
    session_info = await slim2.create_session(
        slim_bindings.PySessionConfiguration.RequestResponse(),
    )

    # messages
    pub_msg = str.encode("thisistherequest")
    res_msg = str.encode("thisistheresponse")

    async with slim1, slim2:
        # create background task for slim1
        async def background_task():
            try:
                # wait for message from any new session
                recv_session, _ = await slim1.receive()

                # receive message from session
                recv_session, msg_rcv = await slim1.receive(session=recv_session.id)

                # make sure the message is correct
                assert msg_rcv == bytes(pub_msg)

                # reply to the session
                await slim1.publish_to(recv_session, res_msg)
            except Exception as e:
                print("Error receiving message on slim1:", e)

        t = asyncio.create_task(background_task())

        # send a request and expect a response in slim2
        session_info, message = await slim2.request_reply(
            session_info, pub_msg, org, ns, agent1
        )

        # check if the message is correct
        assert message == bytes(res_msg)

        # wait for task to finish
        await t
