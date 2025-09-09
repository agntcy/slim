# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime

import pytest
from common import create_slim

import slim_bindings


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12356"], indirect=True)
async def test_request_reply(server):
    org = "cisco"
    ns = "default"
    app1 = "slim1"

    name1 = slim_bindings.PyName(org, ns, app1)

    # create new slim object
    slim1 = await create_slim(name1, "secret")

    # Connect to the service and subscribe for the local name
    _ = await slim1.connect(
        {"endpoint": "http://127.0.0.1:12356", "tls": {"insecure": True}}
    )

    # create second local app
    name2 = slim_bindings.PyName(org, ns, "slim2")
    slim2 = await create_slim(name2, "secret")

    # Connect to SLIM server
    _ = await slim2.connect(
        {"endpoint": "http://127.0.0.1:12356", "tls": {"insecure": True}}
    )

    # set route
    await slim2.set_route(name1)

    # create request/reply session with default config
    session_info = await slim2.create_session(
        slim_bindings.PySessionConfiguration.FireAndForget(
            timeout=datetime.timedelta(seconds=1), max_retries=3, sticky=False
        )
    )

    # messages
    pub_msg = str.encode("thisistherequest")
    res_msg = str.encode("thisistheresponse")

    # Test with reply
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
                assert recv_session.destination_name.equal_without_id(name1)

                # reply to the session
                await slim1.publish_to(recv_session, res_msg)
            except Exception as e:
                print("Error receiving message on slim1:", e)

        t = asyncio.create_task(background_task())

        # send a request and expect a response in slim2
        session_info, message = await slim2.request_reply(session_info, pub_msg, name1)

        # check if the message is correct
        assert message == bytes(res_msg)
        assert session_info.destination_name.equal_without_id(name2)

        # wait for task to finish
        await t

    # Test without reply
    async with slim1, slim2:
        # send a request. No one is listening, so expect a timeout exception
        with pytest.raises(asyncio.TimeoutError):
            session_info, message = await slim2.request_reply(
                session_info,
                pub_msg,
                name1,
                timeout=datetime.timedelta(seconds=5),
            )
