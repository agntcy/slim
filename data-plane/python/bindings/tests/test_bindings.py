# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""
Integration tests for the slim_bindings Python layer.

These tests exercise:
- End-to-end PointToPoint session creation, message publish/reply, and cleanup.
- Session configuration retrieval and default session configuration propagation.
- Usage of the high-level Slim wrapper (PySession helper methods).
- Automatic client reconnection after a server restart.
- Error handling when targeting a non-existent subscription.

Authentication is simplified by using SharedSecret identity provider/verifier
pairs. Network operations run against an in-process server fixture defined
in tests.conftest.
"""

import asyncio
import datetime

import pytest
from common import create_slim, create_svc

import slim_bindings
from slim_bindings._slim_bindings import (
    connect,
    create_session,
    delete_session,
    disconnect,
    get_message,
    listen_for_session,
    publish,
    run_server,
    set_route,
    stop_server,
    subscribe,
)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12344", None], indirect=True)
async def test_end_to_end(server):
    """Full round-trip:
    - Two services connect (Alice, Bob)
    - Subscribe & route setup
    - PointToPoint session creation (Alice -> Bob)
    - Publish + receive + reply
    - Validate session IDs, payload integrity
    - Test error behavior after deleting session
    - Disconnect cleanup
    """
    alice_name = slim_bindings.Name("org", "default", "alice_e2e")
    bob_name = slim_bindings.Name("org", "default", "bob_e2e")

    # create 2 clients, Alice and Bob
    svc_alice = await create_svc(alice_name, local_service=server.local_service)
    svc_bob = await create_svc(bob_name, local_service=server.local_service)

    # connect to the service
    if server.local_service:
        conn_id_alice = await connect(
            svc_alice,
            {"endpoint": "http://127.0.0.1:12344", "tls": {"insecure": True}},
        )
        conn_id_bob = await connect(
            svc_bob,
            {"endpoint": "http://127.0.0.1:12344", "tls": {"insecure": True}},
        )

        # subscribe alice and bob
        alice_name = slim_bindings.Name("org", "default", "alice_e2e", id=svc_alice.id)
        bob_name = slim_bindings.Name("org", "default", "bob_e2e", id=svc_bob.id)
        await subscribe(svc_alice, alice_name, conn_id_alice)
        await subscribe(svc_bob, bob_name, conn_id_bob)

        await asyncio.sleep(1)

        # set routes
        await set_route(svc_alice, bob_name, conn_id_alice)

    await asyncio.sleep(1)
    print(alice_name)
    print(bob_name)

    # create point to point session
    session_context_alice = await create_session(
        svc_alice, bob_name, slim_bindings.SessionConfiguration.PointToPoint()
    )

    # send msg from Alice to Bob
    msg = [1, 2, 3, 4, 5, 6, 7, 8, 9]
    await publish(session_context_alice, 1, msg, name=bob_name)

    # receive session from Alice
    session_context_bob = await listen_for_session(svc_bob)

    # Receive message from Alice
    message_ctx, msg_rcv = await get_message(session_context_bob)

    # make sure the session id corresponds
    assert session_context_bob.id == session_context_alice.id

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # reply to Alice
    await publish(session_context_bob, 1, msg_rcv, message_ctx=message_ctx)

    # wait for message
    message_context, msg_rcv = await get_message(session_context_alice)

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # delete sessions
    await delete_session(svc_alice, session_context_alice)
    await delete_session(svc_bob, session_context_bob)

    # try to send a message after deleting the session - this should raise an exception
    try:
        await publish(session_context_alice, 1, msg, name=bob_name)
    except Exception as e:
        assert "session closed" in str(e), f"Unexpected error message: {str(e)}"

    if server.local_service:
        # disconnect alice
        await disconnect(svc_alice, conn_id_alice)

        # disconnect bob
        await disconnect(svc_bob, conn_id_bob)

    # try to delete a random session, we should get an exception
    try:
        await delete_session(svc_alice, session_context_alice)
    except Exception as e:
        assert "session closed" in str(e)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12345"], indirect=True)
async def test_slim_wrapper(server):
    """Exercise high-level Slim + PySession convenience API:
    - Instantiate two Slim instances
    - Connect & establish routing
    - Create PointToPoint session and publish
    - Receive via listen_for_session + get_message
    - Validate src/dst/session_type invariants
    - Reply using publish_to helper
    - Ensure errors after session deletion are surfaced
    """
    name1 = slim_bindings.Name("org", "default", "slim1")
    name2 = slim_bindings.Name("org", "default", "slim2")

    # create new slim object
    slim1 = await create_slim(name1, local_service=server.local_service)

    if server.local_service:
        # Connect to the service and subscribe for the local name
        _ = await slim1.connect(
            {"endpoint": "http://127.0.0.1:12345", "tls": {"insecure": True}}
        )

    # create second local app
    slim2 = await create_slim(name2, local_service=server.local_service)

    if server.local_service:
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
        name1,
        slim_bindings.SessionConfiguration.PointToPoint(),
    )

    # publish message
    msg = [1, 2, 3, 4, 5, 6, 7, 8, 9]
    await session_context.publish(msg)

    # wait for a new session
    session_context_rec = await slim1.listen_for_session()
    msg_ctx, msg_rcv = await session_context_rec.get_message()

    # make sure the received session is PointToPoint as well
    assert session_context_rec.session_type == slim_bindings.SessionType.PointToPoint

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
        await session_context.publish(msg)
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
    """Test resilience / auto-reconnect:
    - Establish connection and session
    - Exchange a baseline message
    - Stop and restart server
    - Wait for automatic reconnection
    - Publish again and confirm continuity using original session context
    """
    alice_name = slim_bindings.Name("org", "default", "alice_res")
    bob_name = slim_bindings.Name("org", "default", "bob_res")

    svc_alice = await create_svc(alice_name, local_service=server.local_service)
    svc_bob = await create_svc(bob_name, local_service=server.local_service)

    if server.local_service:
        # connect clients and subscribe for messages
        conn_id_alice = await connect(
            svc_alice,
            {"endpoint": "http://127.0.0.1:12346", "tls": {"insecure": True}},
        )
        conn_id_bob = await connect(
            svc_bob,
            {"endpoint": "http://127.0.0.1:12346", "tls": {"insecure": True}},
        )

        alice_name = slim_bindings.Name("org", "default", "alice_res", id=svc_alice.id)
        bob_name = slim_bindings.Name("org", "default", "bob_res", id=svc_bob.id)
        await subscribe(svc_alice, alice_name, conn_id_alice)
        await subscribe(svc_bob, bob_name, conn_id_bob)

        # set routing from Alice to Bob
        await set_route(svc_alice, bob_name, conn_id_alice)

        # Wait for routes to propagate
        await asyncio.sleep(1)

    # create point to point session
    session_context = await create_session(
        svc_alice,
        bob_name,
        slim_bindings.SessionConfiguration.PointToPoint(),
    )

    # send baseline message Alice -> Bob; Bob should first receive a new session then the message
    baseline_msg = [1, 2, 3]
    await publish(session_context, 1, baseline_msg, name=bob_name)

    # Bob waits for new session
    bob_session_ctx = await listen_for_session(svc_bob)
    msg_ctx, received = await get_message(bob_session_ctx)
    assert received == bytes(baseline_msg)
    # session ids should match
    assert bob_session_ctx.id == session_context.id

    # restart the server
    await stop_server(server.service, "127.0.0.1:12346")
    await asyncio.sleep(3)  # allow time for the server to fully shut down
    await run_server(
        server.service, {"endpoint": "127.0.0.1:12346", "tls": {"insecure": True}}
    )
    await asyncio.sleep(2)  # allow time for automatic reconnection

    # test that the message exchange resumes normally after the simulated restart
    test_msg = [4, 5, 6]
    await publish(session_context, 1, test_msg, name=bob_name)
    # Bob should still use the existing session context; just receive next message
    msg_ctx, received = await get_message(bob_session_ctx)
    assert received == bytes(test_msg)

    # delete sessions
    await delete_session(svc_alice, session_context)
    await delete_session(svc_bob, bob_session_ctx)

    # clean up
    await disconnect(svc_alice, conn_id_alice)
    await disconnect(svc_bob, conn_id_bob)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12347"], indirect=True)
async def test_error_on_nonexistent_subscription(server):
    """Validate error path when publishing to an unsubscribed / nonexistent destination:
    - Create only Alice, subscribe her
    - Publish message addressed to Bob (not connected)
    - Expect an error surfaced (no matching subscription)
    """
    name = slim_bindings.Name("org", "default", "alice_nonsub")

    svc_alice = await create_svc(name, local_service=server.local_service)

    if server.local_service:
        # connect client and subscribe for messages
        conn_id_alice = await connect(
            svc_alice,
            {"endpoint": "http://127.0.0.1:12347", "tls": {"insecure": True}},
        )
        alice_class = slim_bindings.Name(
            "org", "default", "alice_nonsub", id=svc_alice.id
        )
        await subscribe(svc_alice, alice_class, conn_id_alice)

    # create Bob's name, but do not instantiate or subscribe Bob
    bob_name = slim_bindings.Name("org", "default", "bob_nonsub")

    # create point to point session (Alice only)
    session_context = await create_session(
        svc_alice,
        bob_name,
        slim_bindings.SessionConfiguration.PointToPoint(),
    )

    # publish a message from Alice intended for Bob (who is not there)
    msg = [7, 8, 9]
    await publish(session_context, 1, msg, name=bob_name)

    # attempt to receive on Alice's session context; since Bob does not exist, no message should arrive
    # and we shohuld also get an error coming from SLIM
    try:
        _, src, received = await asyncio.wait_for(
            listen_for_session(svc_alice), timeout=5
        )
    except asyncio.TimeoutError:
        pytest.fail("timed out waiting for error message on receive channel")
    except Exception as e:
        assert "no matching found" in str(e), f"Unexpected error message: {str(e)}"
    else:
        pytest.fail(f"Expected an exception, but received message: {received}")

    # delete session
    await delete_session(svc_alice, session_context)

    # clean up
    await disconnect(svc_alice, conn_id_alice)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12345", None], indirect=True)
async def test_listen_for_session_timeout(server):
    """Test that listen_for_session times out appropriately when no session is available."""
    alice_name = slim_bindings.Name("org", "default", "alice_timeout")

    svc_alice = await create_svc(alice_name, local_service=server.local_service)

    conn_id_alice = None
    if server.local_service:
        # Connect to the service to get connection ID
        conn_id_alice = await connect(
            svc_alice,
            {"endpoint": "http://127.0.0.1:12345", "tls": {"insecure": True}},
        )

    # Test with a short timeout - should raise an exception
    start_time = asyncio.get_event_loop().time()
    timeout_duration = datetime.timedelta(milliseconds=100)

    with pytest.raises(Exception) as exc_info:
        await listen_for_session(svc_alice, timeout_duration)

    elapsed_time = asyncio.get_event_loop().time() - start_time

    # Verify the timeout was respected (allow some tolerance)
    assert 0.08 <= elapsed_time <= 0.2, (
        f"Timeout took {elapsed_time:.3f}s, expected ~0.1s"
    )
    assert (
        "timed out" in str(exc_info.value).lower()
        or "timeout" in str(exc_info.value).lower()
    )

    # Test with None timeout - should wait indefinitely (we'll interrupt it)
    with pytest.raises(asyncio.TimeoutError):
        await asyncio.wait_for(
            listen_for_session(svc_alice, None),
            timeout=0.1,  # Our own timeout to prevent hanging
        )

    # Clean up
    if conn_id_alice is not None:
        await disconnect(svc_alice, conn_id_alice)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12346", None], indirect=True)
async def test_get_message_timeout(server):
    """Test that get_message times out appropriately when no message is available."""
    alice_name = slim_bindings.Name("org", "default", "alice_msg_timeout")

    # Create service
    svc_alice = await create_svc(alice_name, local_service=server.local_service)

    conn_id_alice = None

    if server.local_service:
        # Connect to the service to get connection ID
        conn_id_alice = await connect(
            svc_alice,
            {"endpoint": "http://127.0.0.1:12346", "tls": {"insecure": True}},
        )

    # Create a session (with dummy peer for timeout testing)
    dummy_peer = slim_bindings.Name("org", "default", "dummy_peer")
    session_context = await create_session(
        svc_alice, dummy_peer, slim_bindings.SessionConfiguration.PointToPoint()
    )

    # Test with a short timeout - should raise an exception
    start_time = asyncio.get_event_loop().time()
    timeout_duration = datetime.timedelta(milliseconds=100)

    with pytest.raises(Exception) as exc_info:
        await get_message(session_context, timeout_duration)

    elapsed_time = asyncio.get_event_loop().time() - start_time

    # Verify the timeout was respected (allow some tolerance)
    assert 0.08 <= elapsed_time <= 0.2, (
        f"Timeout took {elapsed_time:.3f}s, expected ~0.1s"
    )
    assert (
        "timed out" in str(exc_info.value).lower()
        or "timeout" in str(exc_info.value).lower()
    )

    # Test with None timeout - should wait indefinitely (we'll interrupt it)
    with pytest.raises(asyncio.TimeoutError):
        await asyncio.wait_for(
            get_message(session_context, None),
            timeout=0.1,  # Our own timeout to prevent hanging
        )

    # Clean up
    await delete_session(svc_alice, session_context)
    if conn_id_alice is not None:
        await disconnect(svc_alice, conn_id_alice)
