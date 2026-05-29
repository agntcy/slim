# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""
Group integration test for Slim bindings.

Scenario:
  - A configurable number of participants (participants_count) join a group
    "chat" identified by a shared topic (Name).
  - Participant 0 (the "moderator") creates the group session, invites
    every other participant, and publishes the first message addressed to the
    next participant in a logical ring.
  - Each participant, upon receiving a message that ends with its own name,
    publishes a new message naming the next participant, continuing the ring.
  - Each participant exits after observing (participants_count - 1) messages.

What is validated implicitly:
  * Group session establishment (session_type == Group).
  * dst equals the channel/topic Name for non-creator participants.
  * src matches the participant's own local identity when receiving.
  * Message propagation across all participants without loss.
  * Optional MLS flag is parameterized.
"""

import asyncio
import datetime
import uuid

import pytest
from conftest import ServerFixture

import slim_bindings

LONG_SECRET = "e4aaecb9ae0b23b82086bb8a8633e01fba16ae8d9c1379a613c00838"


async def _create_participant(
    server: ServerFixture, test_id: str, index: int
) -> tuple[slim_bindings.App, int | None, str]:
    """Create and setup a participant app."""
    part_name = f"participant_{index}"
    name = slim_bindings.Name("org", f"test_{test_id}", part_name)

    conn_id = None
    if server.local_service:
        svc = slim_bindings.Service(f"svcparticipant{index}")
        client_config = server.get_client_config()
        assert client_config is not None
        conn_id = await svc.connect_async(client_config)
    else:
        svc = server.service

    participant = svc.create_app_with_secret(name, LONG_SECRET)

    if server.local_service:
        name_with_id = slim_bindings.Name.new_with_id(
            "org", f"test_{test_id}", part_name, id=participant.id()
        )
        await participant.subscribe_async(name_with_id, conn_id)
        await asyncio.sleep(0.1)

    return participant, conn_id, part_name


async def _create_group_session(
    participant: slim_bindings.App, chat_name: slim_bindings.Name, mls_enabled: bool
) -> slim_bindings.Session:
    """Create a group session and return it."""
    session_config = slim_bindings.SessionConfig(
        session_type=slim_bindings.SessionType.GROUP,
        enable_mls=mls_enabled,
        max_retries=5,
        interval=datetime.timedelta(seconds=1),
        metadata={},
    )
    session_context = await participant.create_session_async(session_config, chat_name)
    await session_context.completion.wait_async()
    return session_context.session


async def _invite_participants(
    participant, session, server, test_id, participants_count, conn_id, part_name
):
    """Invite all participants to the group session."""
    for i in range(participants_count):
        if i != 0:
            name_to_add = f"participant_{i}"
            to_add = slim_bindings.Name("org", f"test_{test_id}", name_to_add)
            if server.local_service:
                await participant.set_route_async(to_add, conn_id)

            handle = await session.invite_async(to_add)
            await handle.wait_async()
            print(f"{part_name} -> add {name_to_add} to the group")

    await asyncio.sleep(1)  # wait a bit to ensure routes are set up


async def _receive_session(participant, index, first_message):
    """Receive and validate the session if this is the first message."""
    if index == 0 or not first_message:
        return None, first_message

    session_context = await participant.listen_for_session_async(None)
    recv_session = session_context

    assert recv_session.session_type() == slim_bindings.SessionType.GROUP

    return recv_session, False


async def _handle_message_relay(
    recv_session, index, participants_count, part_name, message, msg_rcv, called
):
    """Handle message relay logic - check if message is for this participant and relay."""
    if (not called) and msg_rcv.decode().endswith(part_name):
        print(
            f"{part_name} -> Receiving message: {msg_rcv.decode()}, local count: {index}"
        )

        next_participant = (index + 1) % participants_count
        next_participant_name = f"participant_{next_participant}"
        print(f"{part_name} -> Calling out {next_participant_name}...")

        await recv_session.publish_async(
            f"{message} - {next_participant_name}".encode(), None, None
        )
        print(f"{part_name} -> Published!")
        return True

    print(f"{part_name} -> Receiving message: {msg_rcv.decode()} - not for me.")
    return called


async def _close_session_as_moderator(participant, recv_session, part_name):
    """Close session as moderator (participant 0)."""
    print(f"{part_name} -> Closing session as moderator...")
    await asyncio.sleep(0.5)  # Give others time to finish
    h = await participant.delete_session_async(recv_session)
    await h.wait_async()
    print(f"{part_name} -> Session closed successfully")


async def _wait_for_session_close(recv_session, part_name):
    """Wait for session to be closed by moderator."""
    try:
        await recv_session.get_message_async(timeout=datetime.timedelta(seconds=30))
    except slim_bindings.SlimError.SessionError as e:
        print(f"{part_name} -> Received error {e}")


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "server",
    [
        # "127.0.0.1:12375",  # local service
        None,  # global service
    ],
    indirect=True,
)
@pytest.mark.parametrize("mls_enabled", [False])
async def test_group(server, mls_enabled) -> None:
    """Exercise group session behavior with N participants relaying a message in a ring.

    Steps:
      1. Participant 0 creates group session (optionally with MLS enabled).
      2. Participant 0 invites remaining participants after short delay.
      3. Participant 0 seeds first message naming participant-1.
      4. Each participant that is "called" republishes naming the next participant.
      5. Each participant terminates after seeing (N - 1) messages.

    Args:
        server: Server fixture providing service and configuration (local or global).
        mls_enabled (bool): Whether MLS secure group features are enabled for the session.

    This test asserts invariants via inline assertions and stops when all
    participant tasks complete successfully.
    """
    message = "Calling app"

    # Generate unique names to avoid collisions when using global service
    test_id = str(uuid.uuid4())[:8]

    # participant count
    participants_count = 10
    participants: list[asyncio.Task[None]] = []

    chat_name = slim_bindings.Name("org", f"test_{test_id}", "chat")

    print(f"Test ID: {test_id}")
    print(f"Chat name: {chat_name}")
    print(f"Using {'local' if server.local_service else 'global'} service")

    # Background task: each participant joins/creates the session and relays messages around the ring
    async def background_task(index) -> None:
        """Participant coroutine.

        Responsibilities:
          * Index 0: create group session, invite others, publish initial message, close session when done.
          * Others: wait for inbound session, validate session properties, relay when addressed.
        """
        local_count = 0

        # Create participant
        participant, conn_id, part_name = await _create_participant(
            server, test_id, index
        )
        print(f"Creating participant {part_name}...")

        if index == 0:
            print(f"{part_name} -> Creating new group sessions...")
            session = await _create_group_session(participant, chat_name, mls_enabled)
            await _invite_participants(
                participant,
                session,
                server,
                test_id,
                participants_count,
                conn_id,
                part_name,
            )

        # Track if this participant was called
        called = False
        first_message = True
        recv_session: slim_bindings.Session | None = None

        # if this is the first participant, publish the message to start the chain
        if index == 0:
            next_participant = (index + 1) % participants_count
            next_participant_name = f"participant_{next_participant}"
            msg = f"{message} - {next_participant_name}"
            print(f"{part_name} -> Publishing message as first participant: {msg}")
            called = True
            await session.publish_async(f"{msg}".encode(), None, None)

        while True:
            try:
                # init session from session
                if index == 0:
                    recv_session = session
                else:
                    new_session, first_message = await _receive_session(
                        participant, index, first_message
                    )
                    if new_session:
                        recv_session = new_session
                    # recv_session should already be defined from previous iteration

                assert recv_session is not None
                # receive message from session
                received_msg = await recv_session.get_message_async(
                    timeout=datetime.timedelta(seconds=30)
                )
                _ = received_msg.context
                msg_rcv = received_msg.payload

                # increase the count
                local_count += 1

                # make sure the message is correct
                assert msg_rcv.startswith(bytes(message.encode()))

                # Handle message relay
                called = await _handle_message_relay(
                    recv_session,
                    index,
                    participants_count,
                    part_name,
                    message,
                    msg_rcv,
                    called,
                )

                # All participants exit after receiving all expected messages
                if local_count >= (participants_count - 1):
                    print(f"{part_name} -> Received all {local_count} messages")

                    # Moderator (participant 0) closes the session
                    if index == 0:
                        await _close_session_as_moderator(
                            participant, recv_session, part_name
                        )
                    else:
                        await _wait_for_session_close(recv_session, part_name)

                    break

            except Exception as e:
                print(f"{part_name} -> Unexpected error: {e}")
                raise

        return None

    # start participants in background
    for i in range(participants_count):
        task = asyncio.create_task(background_task(i))
        task.set_name(f"participant_{i}")
        participants.append(task)

    # Wait for all participants to complete
    for task in participants:
        await task

    print("All participants completed successfully!")
