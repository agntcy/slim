# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio

import pytest
from common import create_slim, create_svc

import slim_bindings


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12344"], indirect=True)
async def test_end_to_end(server):
    alice_name = slim_bindings.PyName("org", "default", "alice")
    bob_name = slim_bindings.PyName("org", "default", "bob")

    # create 2 clients, Alice and Bob
    svc_alice = await create_svc(alice_name, "secret")
    svc_bob = await create_svc(bob_name, "secret")

    # connect to the service
    conn_id_alice = await slim_bindings.connect(
        svc_alice,
        {"endpoint": "http://127.0.0.1:12344", "tls": {"insecure": True}},
    )
    conn_id_bob = await slim_bindings.connect(
        svc_bob,
        {"endpoint": "http://127.0.0.1:12344", "tls": {"insecure": True}},
    )

    # subscribe alice and bob
    alice_name = slim_bindings.PyName("org", "default", "alice", id=svc_alice.id)
    bob_name = slim_bindings.PyName("org", "default", "bob", id=svc_bob.id)
    await slim_bindings.subscribe(svc_alice, conn_id_alice, alice_name)
    await slim_bindings.subscribe(svc_bob, conn_id_bob, bob_name)

    await asyncio.sleep(1)

    # set routes
    await slim_bindings.set_route(svc_alice, conn_id_alice, bob_name)

    # create fire and forget session
    session_info = await slim_bindings.create_session(
        svc_alice, slim_bindings.PySessionConfiguration.FireAndForget()
    )

    # send msg from Alice to Bob
    msg = [1, 2, 3, 4, 5, 6, 7, 8, 9]
    await slim_bindings.publish(svc_alice, session_info, 1, msg, bob_name)

    # receive message from Alice
    session_info_ret, msg_rcv = await slim_bindings.receive(svc_bob)

    # make sure the session id corresponds
    assert session_info_ret.id == session_info.id

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # reply to Alice
    await slim_bindings.publish(svc_bob, session_info_ret, 1, msg_rcv)

    # wait for message
    session_info_ret, msg_rcv = await slim_bindings.receive(svc_alice)

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # delete sessions
    await slim_bindings.delete_session(svc_alice, session_info.id)
    await slim_bindings.delete_session(svc_bob, session_info.id)

    # try to send a message after deleting the session - this should raise an exception
    try:
        await slim_bindings.publish(svc_alice, session_info, 1, msg, bob_name)
    except Exception as e:
        assert "session not found" in str(e), f"Unexpected error message: {str(e)}"

    # disconnect alice
    await slim_bindings.disconnect(svc_alice, conn_id_alice)

    # disconnect bob
    await slim_bindings.disconnect(svc_bob, conn_id_bob)

    # try to delete a random session, we should get an exception
    try:
        await slim_bindings.delete_session(svc_alice, 123456789)
    except Exception as e:
        assert "session not found" in str(e)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12344"], indirect=True)
async def test_session_config(server):
    alice_name = slim_bindings.PyName("org", "default", "alice")

    stream_name = slim_bindings.PyName("org", "default", "stream")

    # create svc
    svc = await create_svc(alice_name, "secret")

    # create fire and forget session
    session_config = slim_bindings.PySessionConfiguration.FireAndForget()
    session_info = await slim_bindings.create_session(svc, session_config)

    # get session configuration
    session_config_ret = await slim_bindings.get_session_config(svc, session_info.id)

    # check if the session config is correct
    assert isinstance(
        session_config, slim_bindings.PySessionConfiguration.FireAndForget
    )
    assert session_config == session_config_ret, (
        f"session config are not equal: {session_config} vs {session_config_ret}"
    )

    # check default values
    await slim_bindings.set_default_session_config(
        svc,
        session_config,
    )

    # get default
    session_config_ret = await slim_bindings.get_default_session_config(
        svc, slim_bindings.PySessionType.FIRE_AND_FORGET
    )

    # check if the session config is correct
    assert isinstance(
        session_config_ret, slim_bindings.PySessionConfiguration.FireAndForget
    )
    assert session_config == session_config_ret, (
        f"session config are not equal: {session_config} vs {session_config_ret}"
    )

    # Streaming session
    session_config = slim_bindings.PySessionConfiguration.Streaming(
        slim_bindings.PySessionDirection.SENDER, stream_name, False, 12345
    )

    session_info = await slim_bindings.create_session(svc, session_config)
    session_config_ret = await slim_bindings.get_session_config(svc, session_info.id)
    # check if the session config is correct
    assert isinstance(
        session_config_ret, slim_bindings.PySessionConfiguration.Streaming
    )
    assert session_config == session_config_ret

    # check default values

    # This session direction
    session_config = slim_bindings.PySessionConfiguration.Streaming(
        slim_bindings.PySessionDirection.SENDER, stream_name, False, 12345
    )

    # Try to set a sender direction as default session. We should get an error, as we are trying to
    # set a sender as default session
    try:
        await slim_bindings.set_default_session_config(
            svc,
            session_config,
        )
    except Exception as e:
        assert "cannot change session direction" in str(e), (
            f"Unexpected error message: {str(e)}"
        )

    # Use a receiver direction
    session_config = slim_bindings.PySessionConfiguration.Streaming(
        slim_bindings.PySessionDirection.RECEIVER, stream_name, False, 12345
    )
    await slim_bindings.set_default_session_config(
        svc,
        session_config,
    )

    # get default
    session_config_ret = await slim_bindings.get_default_session_config(
        svc, slim_bindings.PySessionType.STREAMING
    )
    # check if the session config is correct
    assert isinstance(
        session_config_ret, slim_bindings.PySessionConfiguration.Streaming
    )
    assert session_config == session_config_ret


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12345"], indirect=True)
async def test_slim_wrapper(server):
    name1 = slim_bindings.PyName("org", "default", "slim1")
    name2 = slim_bindings.PyName("org", "default", "slim2")

    # create new slim object
    slim1 = await create_slim(name1, "secret")

    # Connect to the service and subscribe for the local name
    _ = await slim1.connect(
        {"endpoint": "http://127.0.0.1:12345", "tls": {"insecure": True}}
    )

    # create second local app
    slim2 = await create_slim(name2, "secret")

    # Connect to SLIM server
    _ = await slim2.connect(
        {"endpoint": "http://127.0.0.1:12345", "tls": {"insecure": True}}
    )

    # Wait for routes to propagate
    await asyncio.sleep(1)

    # set route
    await slim2.set_route(name1)

    # create session
    session_info = await slim2.create_session(
        slim_bindings.PySessionConfiguration.FireAndForget()
    )

    async with slim1, slim2:
        # publish message
        msg = [1, 2, 3, 4, 5, 6, 7, 8, 9]
        await slim2.publish(session_info, msg, name1)

        # wait for a new session
        session_info_rec, _ = await slim1.receive()

        # new session received! listen for the message
        session_info_rec, msg_rcv = await slim1.receive(session=session_info_rec.id)

        # check if the message is correct
        assert msg_rcv == bytes(msg)

        # make sure the session info is correct
        assert session_info.id == session_info_rec.id

        # reply to Alice
        await slim1.publish_to(session_info_rec, msg_rcv)

        # wait for message
        _, msg_rcv = await slim2.receive(session=session_info.id)

        # check if the message is correct
        assert msg_rcv == bytes(msg)

    # delete sessions
    await slim1.delete_session(session_info.id)
    await slim2.delete_session(session_info.id)

    # try to send a message after deleting the session - this should raise an exception
    try:
        await slim1.publish(session_info, msg, name1)
    except Exception as e:
        assert "session not found" in str(e), f"Unexpected error message: {str(e)}"

    # try to delete a random session, we should get an exception
    try:
        await slim1.delete_session(123456789)
    except Exception as e:
        assert "session not found" in str(e), f"Unexpected error message: {str(e)}"


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12346"], indirect=True)
async def test_auto_reconnect_after_server_restart(server):
    alice_name = slim_bindings.PyName("org", "default", "alice")
    bob_name = slim_bindings.PyName("org", "default", "bob")

    svc_alice = await create_svc(alice_name, "secret")
    svc_bob = await create_svc(bob_name, "secret")

    # connect clients and subscribe for messages
    conn_id_alice = await slim_bindings.connect(
        svc_alice,
        {"endpoint": "http://127.0.0.1:12346", "tls": {"insecure": True}},
    )
    conn_id_bob = await slim_bindings.connect(
        svc_bob,
        {"endpoint": "http://127.0.0.1:12346", "tls": {"insecure": True}},
    )

    alice_name = slim_bindings.PyName("org", "default", "alice", id=svc_alice.id)
    bob_name = slim_bindings.PyName("org", "default", "bob", id=svc_bob.id)
    await slim_bindings.subscribe(svc_alice, conn_id_alice, alice_name)
    await slim_bindings.subscribe(svc_bob, conn_id_bob, bob_name)

    # Wait for routes to propagate
    await asyncio.sleep(1)

    # set routing from Alice to Bob
    await slim_bindings.set_route(svc_alice, conn_id_alice, bob_name)

    # create fire and forget session
    session_info = await slim_bindings.create_session(
        svc_alice, slim_bindings.PySessionConfiguration.FireAndForget()
    )

    # verify baseline message exchange before the simulated server restart
    baseline_msg = [1, 2, 3]
    await slim_bindings.publish(svc_alice, session_info, 1, baseline_msg, bob_name)

    _, received = await slim_bindings.receive(svc_bob)
    assert received == bytes(baseline_msg)

    # restart the server
    await slim_bindings.stop_server(server, "127.0.0.1:12346")
    await asyncio.sleep(3)  # allow time for the server to fully shut down
    await slim_bindings.run_server(
        server, {"endpoint": "127.0.0.1:12346", "tls": {"insecure": True}}
    )
    await asyncio.sleep(2)  # allow time for automatic reconnection

    # test that the message exchange resumes normally after the simulated restart
    test_msg = [4, 5, 6]
    await slim_bindings.publish(svc_alice, session_info, 1, test_msg, bob_name)
    _, received = await slim_bindings.receive(svc_bob)
    assert received == bytes(test_msg)

    # clean up
    await slim_bindings.disconnect(svc_alice, conn_id_alice)
    await slim_bindings.disconnect(svc_bob, conn_id_bob)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12347"], indirect=True)
async def test_error_on_nonexistent_subscription(server):
    name = slim_bindings.PyName("org", "default", "alice")

    svc_alice = await create_svc(name, "secret")

    # connect client and subscribe for messages
    conn_id_alice = await slim_bindings.connect(
        svc_alice,
        {"endpoint": "http://127.0.0.1:12347", "tls": {"insecure": True}},
    )
    alice_class = slim_bindings.PyName("org", "default", "alice", id=svc_alice.id)
    await slim_bindings.subscribe(svc_alice, conn_id_alice, alice_class)

    # create fire and forget session
    session_info = await slim_bindings.create_session(
        svc_alice, slim_bindings.PySessionConfiguration.FireAndForget()
    )

    # create Bob's name, but do not instantiate or subscribe Bob
    bob_name = slim_bindings.PyName("org", "default", "bob")

    # publish a message from Alice intended for Bob (who is not there)
    msg = [7, 8, 9]
    await slim_bindings.publish(svc_alice, session_info, 1, msg, bob_name)

    # an exception should be raised on receive
    try:
        _, src, received = await asyncio.wait_for(
            slim_bindings.receive(svc_alice), timeout=5
        )
    except asyncio.TimeoutError:
        pytest.fail("timed out waiting for error message on receive channel")
    except Exception as e:
        assert "no matching found" in str(e), f"Unexpected error message: {str(e)}"
    else:
        pytest.fail(f"Expected an exception, but received message: {received}")

    # clean up
    await slim_bindings.disconnect(svc_alice, conn_id_alice)
