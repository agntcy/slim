# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""
Integration tests for the slim_bindings Python layer.

These tests exercise:
- End-to-end PointToPoint session creation, message publish/reply, and cleanup.
- Session configuration retrieval and default session configuration propagation.
- Automatic client reconnection after a server restart.
- Error handling when targeting a non-existent subscription.

Authentication is simplified by using SharedSecret identity provider/verifier
pairs. Network operations run against an in-process server fixture defined
in tests.conftest.
"""

import asyncio
import datetime

import pytest

import slim_bindings

LONG_SECRET = "e4aaecb9ae0b23b82086bb8a8633e01fba16ae8d9c1379a613c00838"


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12344"], indirect=True)
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

    # connect to the service
    conn_id_alice = None
    conn_id_bob = None
    if server.local_service:
        svc_alice = slim_bindings.Service("svcalice")
        svc_bob = slim_bindings.Service("svcbob")

        conn_id_alice = await svc_alice.connect_async(server.get_client_config())
        conn_id_bob = await svc_bob.connect_async(server.get_client_config())
    else:
        svc_alice = server.service
        svc_bob = server.service

    # create 2 apps, Alice and Bob
    app_alice = svc_alice.create_app_with_secret(alice_name, LONG_SECRET)
    app_bob = svc_bob.create_app_with_secret(bob_name, LONG_SECRET)

    if server.local_service:
        # subscribe alice and bob
        alice_name = slim_bindings.Name.new_with_id(
            "org", "default", "alice_e2e", id=app_alice.id()
        )
        bob_name = slim_bindings.Name.new_with_id(
            "org", "default", "bob_e2e", id=app_bob.id()
        )

        await app_alice.subscribe_async(alice_name, conn_id_alice)
        await app_bob.subscribe_async(bob_name, conn_id_bob)

        await asyncio.sleep(1)

        # set routes
        await app_alice.set_route_async(bob_name, conn_id_alice)

    await asyncio.sleep(1)
    print(alice_name.as_string())
    print(bob_name.as_string())

    # create point to point session
    session_context_alice = await app_alice.create_session_async(
        slim_bindings.SessionConfig(
            session_type=slim_bindings.SessionType.POINT_TO_POINT,
            enable_mls=False,
            max_retries=5,
            interval=datetime.timedelta(seconds=1),
            metadata={},
        ),
        bob_name,
    )

    # Wait for the session handshake to fully complete
    await session_context_alice.completion.wait_async()

    # send msg from Alice to Bob
    msg = [1, 2, 3, 4, 5, 6, 7, 8, 9]
    handle = await session_context_alice.session.publish_async(bytes(msg), None, None)

    # Wait for ack to be received
    await handle.wait_async()

    # receive session from Alice
    session_context_bob = await app_bob.listen_for_session_async(
        timeout=datetime.timedelta(seconds=5)
    )

    # Receive message from Alice
    received_msg = await session_context_bob.get_message_async(
        timeout=datetime.timedelta(seconds=5)
    )
    message_ctx = received_msg.context
    msg_rcv = received_msg.payload

    # make sure the session id corresponds
    assert (
        session_context_bob.session_id() == session_context_alice.session.session_id()
    )

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # reply to Alice
    handle = await session_context_bob.publish_to_async(
        message_ctx, msg_rcv, None, None
    )

    # Wait for ack
    await handle.wait_async()

    # wait for message
    received_msg = await session_context_alice.session.get_message_async(
        datetime.timedelta(seconds=5)
    )
    _ = received_msg.context
    msg_rcv = received_msg.payload

    # check if the message is correct
    assert msg_rcv == bytes(msg)

    # delete both sessions by deleting bob
    await app_alice.delete_session_async(session_context_alice.session)

    # try to send a message after deleting the session - this should raise an exception
    try:
        await session_context_alice.session.publish_async(bytes(msg), None, None)
    except Exception as e:
        assert "closed" in str(e).lower(), f"Unexpected error message: {str(e)}"

    if conn_id_alice is not None:
        # disconnect alice
        svc_alice.disconnect(conn_id_alice)

    try:
        await app_alice.delete_session_async(session_context_alice.session)
    except Exception as e:
        assert "closed" in str(e).lower()


@pytest.mark.asyncio
@pytest.mark.parametrize("server", [None], indirect=True)
async def test_auto_reconnect_after_server_restart(server):
    """Test resilience / auto-reconnect:
    - Establish connection and session
    - Exchange a baseline message
    - Stop and restart server
    - Wait for automatic reconnection
    - Publish again and confirm continuity using original session context
    """
    endpoint = "127.0.0.1:12346"

    svc_server = slim_bindings.Service("svcserver")
    svc_alice = slim_bindings.Service("svcalice")
    svc_bob = slim_bindings.Service("svcbob")

    server_conf = slim_bindings.new_insecure_server_config(endpoint)
    await svc_server.run_server_async(server_conf)

    client_conf = slim_bindings.new_insecure_client_config("http://" + endpoint)
    conn_id_alice = await svc_alice.connect_async(client_conf)
    conn_id_bob = await svc_bob.connect_async(client_conf)

    alice_name = slim_bindings.Name("org", "default", "alice_res")
    bob_name = slim_bindings.Name("org", "default", "bob_res")

    # create 2 apps, Alice and Bob
    app_alice = svc_alice.create_app_with_secret(alice_name, LONG_SECRET)
    app_bob = svc_bob.create_app_with_secret(bob_name, LONG_SECRET)

    # subscribe alice and bob
    alice_name = slim_bindings.Name.new_with_id(
        "org", "default", "alice_res", id=app_alice.id()
    )
    bob_name = slim_bindings.Name.new_with_id(
        "org", "default", "bob_res", id=app_bob.id()
    )

    await app_alice.subscribe_async(alice_name, conn_id_alice)
    await app_bob.subscribe_async(bob_name, conn_id_bob)

    await asyncio.sleep(1)

    # set routes
    await app_alice.set_route_async(bob_name, conn_id_alice)

    await asyncio.sleep(1)

    # create point to point session
    session_context_alice = await app_alice.create_session_async(
        slim_bindings.SessionConfig(
            session_type=slim_bindings.SessionType.POINT_TO_POINT,
            enable_mls=False,
            max_retries=5,
            interval=datetime.timedelta(seconds=1),
            metadata={},
        ),
        bob_name,
    )

    # Wait for the session handshake to fully complete
    await session_context_alice.completion.wait_async()

    # send baseline message Alice -> Bob; Bob should first receive a new session then the message
    baseline_msg = [1, 2, 3]
    handle = await session_context_alice.session.publish_async(
        bytes(baseline_msg), None, None
    )
    await handle.wait_async()

    # Bob waits for new session
    bob_session_ctx = await app_bob.listen_for_session_async(
        timeout=datetime.timedelta(seconds=5)
    )
    received_msg = await bob_session_ctx.get_message_async(
        timeout=datetime.timedelta(seconds=5)
    )
    _ = received_msg.context
    received = received_msg.payload
    assert received == bytes(baseline_msg)
    # session ids should match
    assert bob_session_ctx.session_id() == session_context_alice.session.session_id()

    # stop the server
    await svc_server.shutdown_async()

    await asyncio.sleep(3)  # allow time for the server to fully shut down

    # start the server
    _ = slim_bindings.Service("svcserver")
    await server.service.run_server_async(
        slim_bindings.new_insecure_server_config("127.0.0.1:12346")
    )
    await asyncio.sleep(
        3
    )  # allow time for the server to fully shut down and restart, and for clients to reconnect

    # test that the message exchange resumes normally after the simulated restart
    test_msg = [4, 5, 6]
    handle = await session_context_alice.session.publish_async(
        bytes(test_msg), None, None
    )
    await handle.wait_async()

    # Bob should still use the existing session context; just receive next message
    received_msg = await bob_session_ctx.get_message_async(
        timeout=datetime.timedelta(seconds=5)
    )
    _ = received_msg.context
    received = received_msg.payload
    assert received == bytes(test_msg)

    # delete sessions by deleting alice session
    await app_alice.delete_session_async(session_context_alice.session)

    # clean up
    # Disconnect (only needed if we have connections)
    if conn_id_alice is not None:
        svc_alice.disconnect(conn_id_alice)
    if conn_id_bob is not None:
        svc_bob.disconnect(conn_id_bob)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12347"], indirect=True)
async def test_error_on_nonexistent_subscription(server):
    """Validate error path when publishing to an unsubscribed / nonexistent destination:
    - Create only Alice, subscribe her
    - Publish message addressed to Bob (not connected)
    - Expect an error surfaced (no matching subscription)
    """
    alice_name = slim_bindings.Name("org", "default", "alice_nonsub")

    conn_id_alice = None
    if server.local_service:
        svc_alice = slim_bindings.Service("svcalice")

        # connect client and subscribe for messages
        conn_id_alice = await svc_alice.connect_async(server.get_client_config())

        app_alice = svc_alice.create_app_with_secret(alice_name, LONG_SECRET)

        alice_name = slim_bindings.Name.new_with_id(
            "org", "default", "alice_nonsub", id=app_alice.id()
        )
        await app_alice.subscribe_async(alice_name, conn_id_alice)
    else:
        svc_alice = server.service
        app_alice = svc_alice.create_app_with_secret(alice_name, LONG_SECRET)

    # create Bob's name, but do not instantiate or subscribe Bob
    bob_name = slim_bindings.Name("org", "default", "bob_nonsub")

    # create point to point session (Alice only) - should timeout or error since Bob is not there
    with pytest.raises((asyncio.TimeoutError, slim_bindings.SlimError)):
        session = await app_alice.create_session_async(
            slim_bindings.SessionConfig(
                session_type=slim_bindings.SessionType.POINT_TO_POINT,
                enable_mls=False,
                max_retries=None,
                interval=None,
                metadata={},
            ),
            bob_name,
        )

        await session.completion.wait_for_async(datetime.timedelta(seconds=10))

    # clean up
    if conn_id_alice is not None:
        svc_alice.disconnect(conn_id_alice)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12348", None], indirect=True)
async def test_listen_for_session_timeout(server):
    """Test that listen_for_session times out appropriately when no session is available."""
    alice_name = slim_bindings.Name("org", "default", "alice_timeout")

    conn_id_alice = None
    if server.local_service:
        svc_alice = slim_bindings.Service("svcalice")

        # Connect to the service to get connection ID
        conn_id_alice = await svc_alice.connect_async(server.get_client_config())

        app_alice = svc_alice.create_app_with_secret(alice_name, LONG_SECRET)

        alice_name = slim_bindings.Name.new_with_id(
            "org", "default", "alice_timeout", id=app_alice.id()
        )
        await app_alice.subscribe_async(alice_name, conn_id_alice)
    else:
        svc_alice = server.service
        app_alice = svc_alice.create_app_with_secret(alice_name, LONG_SECRET)

    # Test with a short timeout - should raise an exception
    start_time = asyncio.get_event_loop().time()
    timeout = datetime.timedelta(milliseconds=100)

    with pytest.raises(Exception) as exc_info:
        await app_alice.listen_for_session_async(timeout=timeout)

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
            app_alice.listen_for_session_async(timeout=None),
            timeout=0.1,  # Our own timeout to prevent hanging
        )

    # Clean up
    if conn_id_alice is not None:
        svc_alice.disconnect(conn_id_alice)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12349", None], indirect=True)
async def test_get_message_timeout(server):
    """Test that get_message times out appropriately when no message is available."""
    alice_name = slim_bindings.Name("org", "default", "alice_msg_timeout")
    bob_name = slim_bindings.Name("org", "default", "bob_msg_timeout")

    conn_id_alice = None
    conn_id_bob = None
    if server.local_service:
        svc_alice = slim_bindings.Service("svcalice")
        svc_bob = slim_bindings.Service("svcbob")

        conn_id_alice = await svc_alice.connect_async(server.get_client_config())
        conn_id_bob = await svc_bob.connect_async(server.get_client_config())
    else:
        svc_alice = server.service
        svc_bob = server.service

    # create 2 apps, Alice and Bob
    app_alice = svc_alice.create_app_with_secret(alice_name, LONG_SECRET)
    app_bob = svc_bob.create_app_with_secret(bob_name, LONG_SECRET)

    if server.local_service:
        # subscribe alice and bob
        alice_name = slim_bindings.Name.new_with_id(
            "org", "default", "alice_msg_timeout", id=app_alice.id()
        )
        bob_name = slim_bindings.Name.new_with_id(
            "org", "default", "bob_msg_timeout", id=app_bob.id()
        )

        await app_alice.subscribe_async(alice_name, conn_id_alice)
        await app_bob.subscribe_async(bob_name, conn_id_bob)

        await asyncio.sleep(1)

        # set routes
        await app_alice.set_route_async(bob_name, conn_id_alice)

    await asyncio.sleep(1)

    # create point to point session
    session_context_alice = await app_alice.create_session_async(
        slim_bindings.SessionConfig(
            session_type=slim_bindings.SessionType.POINT_TO_POINT,
            enable_mls=False,
            max_retries=5,
            interval=datetime.timedelta(seconds=1),
            metadata={},
        ),
        bob_name,
    )

    # Wait for the session handshake to fully complete
    await session_context_alice.completion.wait_async()

    # Bob waits for new session
    bob_session_ctx = await app_bob.listen_for_session_async(
        timeout=datetime.timedelta(seconds=5)
    )

    # Test with a short timeout - should raise an exception when no message is available
    start_time = asyncio.get_event_loop().time()
    timeout = datetime.timedelta(milliseconds=100)

    with pytest.raises(Exception) as exc_info:
        await bob_session_ctx.get_message_async(timeout=timeout)

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
            bob_session_ctx.get_message_async(timeout=None),
            timeout=0.1,  # Our own timeout to prevent hanging
        )

    # delete sessions
    handle_alice = await app_alice.delete_session_async(session_context_alice.session)
    handle_bob = await app_bob.delete_session_async(bob_session_ctx)

    # wait for deletion
    await handle_alice.wait_async()
    await handle_bob.wait_async()

    # Clean up
    if conn_id_alice is not None:
        svc_alice.disconnect(conn_id_alice)
    if conn_id_bob is not None:
        svc_bob.disconnect(conn_id_bob)
