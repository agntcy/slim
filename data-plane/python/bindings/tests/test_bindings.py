# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime

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

    print(alice_name)
    print(bob_name)

    # create point to point session
    session_context_alice = await slim_bindings.create_session(
        svc_alice, slim_bindings.PySessionConfiguration.Anycast()
    )

    # send msg from Alice to Bob
    msg = [1, 2, 3, 4, 5, 6, 7, 8, 9]
    await slim_bindings.publish(svc_alice, session_context_alice, 1, msg, name=bob_name)

    # receive session from Alice
    session_context_bob = await slim_bindings.listen_for_session(svc_bob)

    # Receive message from Alice
    message_ctx, msg_rcv = await slim_bindings.get_message(svc_bob, session_context_bob)

    # make sure the session id corresponds
    assert session_context_bob.id == session_context_alice.id

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # reply to Alice
    await slim_bindings.publish(
        svc_bob, session_context_bob, 1, msg_rcv, message_ctx=message_ctx
    )

    # wait for message
    message_context, msg_rcv = await slim_bindings.get_message(
        svc_alice, session_context_alice
    )

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # delete sessions
    await slim_bindings.delete_session(svc_alice, session_context_alice)
    await slim_bindings.delete_session(svc_bob, session_context_bob)

    # try to send a message after deleting the session - this should raise an exception
    try:
        await slim_bindings.publish(
            svc_alice, session_context_alice, 1, msg, name=bob_name
        )
    except Exception as e:
        assert "session closed" in str(e), f"Unexpected error message: {str(e)}"

    # disconnect alice
    await slim_bindings.disconnect(svc_alice, conn_id_alice)

    # disconnect bob
    await slim_bindings.disconnect(svc_bob, conn_id_bob)

    # try to delete a random session, we should get an exception
    try:
        await slim_bindings.delete_session(svc_alice, session_context_alice)
    except Exception as e:
        assert "session closed" in str(e)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12344"], indirect=True)
async def test_session_config(server):
    alice_name = slim_bindings.PyName("org", "default", "alice")

    # create svc
    svc = await create_svc(alice_name, "secret")

    # create an anycast session with custom parameters
    session_config = slim_bindings.PySessionConfiguration.Anycast(
        timeout=datetime.timedelta(seconds=2),
    )

    session_config2 = slim_bindings.PySessionConfiguration.Anycast(
        timeout=datetime.timedelta(seconds=3),
    )

    session_context = await slim_bindings.create_session(svc, session_config)

    # get per-session configuration via new API (synchronous method)
    session_config_ret = session_context.session_config

    assert isinstance(session_config_ret, slim_bindings.PySessionConfiguration.Anycast)
    assert session_config == session_config_ret, (
        f"session config mismatch: {session_config} vs {session_config_ret}"
    )
    assert session_config2 != session_config_ret, (
        f"sessions should differ: {session_config2} vs {session_config_ret}"
    )

    # Set the default session configuration (no direct read-back API; validate by creating a new session)
    slim_bindings.set_default_session_config(svc, session_config2)

    # ------------------------------------------------------------------
    # Validate that a session initiated towards this service adopts the new default
    # ------------------------------------------------------------------
    peer_name = slim_bindings.PyName("org", "default", "peer")
    peer_svc = await create_svc(peer_name, "secret")

    # Connect both services to the running server
    conn_id_local = await slim_bindings.connect(
        svc,
        {"endpoint": "http://127.0.0.1:12344", "tls": {"insecure": True}},
    )
    conn_id_peer = await slim_bindings.connect(
        peer_svc,
        {"endpoint": "http://127.0.0.1:12344", "tls": {"insecure": True}},
    )

    # Build fully qualified names (with instance IDs) and subscribe
    local_name_with_id = slim_bindings.PyName("org", "default", "alice", id=svc.id)
    peer_name_with_id = slim_bindings.PyName("org", "default", "peer", id=peer_svc.id)
    await slim_bindings.subscribe(svc, conn_id_local, local_name_with_id)
    await slim_bindings.subscribe(peer_svc, conn_id_peer, peer_name_with_id)

    # Allow propagation
    await asyncio.sleep(0.5)

    # Set route from peer -> local so peer can send directly
    await slim_bindings.set_route(peer_svc, conn_id_peer, local_name_with_id)

    # Peer creates a session (using anycast; its config is irrelevant for local default assertion)
    peer_session_ctx = await slim_bindings.create_session(
        peer_svc, slim_bindings.PySessionConfiguration.Anycast()
    )

    # Send a first message to trigger session creation on local service
    msg = [9, 9, 9]
    await slim_bindings.publish(
        peer_svc, peer_session_ctx, 1, msg, name=local_name_with_id
    )

    # Local service should receive a new session notification
    received_session_ctx = await slim_bindings.listen_for_session(svc)
    received_config = received_session_ctx.session_config

    # Assert that the received session uses the default we set (session_config2)
    assert received_config == session_config2, (
        f"received session config does not match default: {received_config} vs {session_config2}"
    )

    # Basic sanity: message should be retrievable
    _, payload = await slim_bindings.get_message(svc, received_session_ctx)
    assert payload == bytes(msg)

    # Cleanup connections (session deletion is implicit on drop / test end)
    await slim_bindings.disconnect(peer_svc, conn_id_peer)
    await slim_bindings.disconnect(svc, conn_id_local)


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
    session_context = await slim2.create_session(
        slim_bindings.PySessionConfiguration.Anycast()
    )

    # publish message
    msg = [1, 2, 3, 4, 5, 6, 7, 8, 9]
    await session_context.publish_with_destination(msg, name1)

    # wait for a new session
    session_context_rec = await slim1.listen_for_session()
    msg_ctx, msg_rcv = await session_context_rec.get_message()

    # make sure the received session is anycast as well
    assert session_context_rec.session_type == slim_bindings.PySessionType.ANYCAST

    # Make sure the dst of the session is None (anycast)
    assert session_context_rec.dst is None

    # Make sure the source is correct
    assert session_context_rec.src == slim1.local_name

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # make sure the session id is correct
    assert session_context.id == session_context_rec.id

    # reply to Alice
    await session_context_rec.publish_to(msg_ctx, msg_rcv)

    # wait for message
    msg_ctx, msg_rcv = await session_context.get_message()

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # delete sessions
    await slim1.delete_session(session_context_rec)
    await slim2.delete_session(session_context)

    # try to send a message after deleting the session - this should raise an exception
    try:
        await session_context.publish_with_destination(msg, name1)
    except Exception as e:
        assert "session closed" in str(e), f"Unexpected error message: {str(e)}"

    # try to delete a random session, we should get an exception
    try:
        await slim1.delete_session(session_context)
    except Exception as e:
        assert "session closed" in str(e), f"Unexpected error message: {str(e)}"


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

    # create point to point session (Anycast)
    session_context = await slim_bindings.create_session(
        svc_alice, slim_bindings.PySessionConfiguration.Anycast()
    )

    # send baseline message Alice -> Bob; Bob should first receive a new session then the message
    baseline_msg = [1, 2, 3]
    await slim_bindings.publish(
        svc_alice, session_context, 1, baseline_msg, name=bob_name
    )

    # Bob waits for new session
    bob_session_ctx = await slim_bindings.listen_for_session(svc_bob)
    msg_ctx, received = await slim_bindings.get_message(svc_bob, bob_session_ctx)
    assert received == bytes(baseline_msg)
    # session ids should match
    assert bob_session_ctx.id == session_context.id

    # restart the server
    await slim_bindings.stop_server(server, "127.0.0.1:12346")
    await asyncio.sleep(3)  # allow time for the server to fully shut down
    await slim_bindings.run_server(
        server, {"endpoint": "127.0.0.1:12346", "tls": {"insecure": True}}
    )
    await asyncio.sleep(2)  # allow time for automatic reconnection

    # test that the message exchange resumes normally after the simulated restart
    test_msg = [4, 5, 6]
    await slim_bindings.publish(svc_alice, session_context, 1, test_msg, name=bob_name)
    # Bob should still use the existing session context; just receive next message
    msg_ctx, received = await slim_bindings.get_message(svc_bob, bob_session_ctx)
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

    # create point to point session (Alice only)
    session_context = await slim_bindings.create_session(
        svc_alice, slim_bindings.PySessionConfiguration.Anycast()
    )

    # create Bob's name, but do not instantiate or subscribe Bob
    bob_name = slim_bindings.PyName("org", "default", "bob")

    # publish a message from Alice intended for Bob (who is not there)
    msg = [7, 8, 9]
    await slim_bindings.publish(svc_alice, session_context, 1, msg, name=bob_name)

    # attempt to receive on Alice's session context; since Bob does not exist, no message should arrive
    # and we shohuld also get an error coming from SLIM
    try:
        _, src, received = await asyncio.wait_for(
            slim_bindings.listen_for_session(svc_alice), timeout=5
        )
    except asyncio.TimeoutError:
        pytest.fail("timed out waiting for error message on receive channel")
    except Exception as e:
        assert "no matching found" in str(e), f"Unexpected error message: {str(e)}"
    else:
        pytest.fail(f"Expected an exception, but received message: {received}")

    # clean up
    await slim_bindings.disconnect(svc_alice, conn_id_alice)
