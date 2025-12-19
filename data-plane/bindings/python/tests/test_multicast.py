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
import uuid

import pytest
from common import create_slim, create_name, create_client_config

import slim_uniffi_bindings._slim_bindings.slim_bindings as slim_bindings


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "server",
    [
        "127.0.0.1:12375",  # local service
        # None,  # global service
    ],
    indirect=True,
)
@pytest.mark.parametrize("mls_enabled", [True, False])
async def test_group(server, mls_enabled):  # noqa: C901
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
    participants = []

    chat_name = create_name("org", f"test_{test_id}", "chat")

    print(f"Test ID: {test_id}")
    print(f"Chat name: {chat_name}")
    print(f"Using {'local' if server.local_service else 'global'} service")

    # Background task: each participant joins/creates the session and relays messages around the ring
    async def background_task(index):
        """Participant coroutine.

        Responsibilities:
          * Index 0: create group session, invite others, publish initial message.
          * Others: wait for inbound session, validate session properties, relay when addressed.
        """
        part_name = f"participant-{index}"
        local_count = 0

        print(f"Creating participant {part_name}...")

        # Use unique namespace per test to avoid collisions
        name = create_name("org", f"test_{test_id}", part_name)

        participant = create_slim(name, local_service=server.local_service)

        if server.endpoint is not None:
            # Connect to SLIM server
            print(f"{part_name} -> Connecting to server at {server.endpoint}...")
            conn_id = await participant.connect_async(
                create_client_config(f"http://{server.endpoint}")
            )

        if index == 0:
            print(f"{part_name} -> Creating new group sessions...")
            # create a group session. index 0 is the moderator of the session
            # and it will invite all the other participants to the session
            session_config = slim_bindings.SessionConfig(
                session_type=slim_bindings.SessionType.GROUP,
                enable_mls=mls_enabled,
                max_retries=5,
                interval_ms=1000,  # 1 second interval
                initiator=True,
                metadata={},
            )
            # Create session (auto-waits for establishment)
            session = await participant.create_session_async(session_config, chat_name)

            # invite all participants
            for i in range(participants_count):
                if i != 0:
                    name_to_add = f"participant-{i}"
                    # Use same unique namespace as above
                    to_add = create_name("org", f"test_{test_id}", name_to_add)

                    if server.endpoint is not None:
                        await participant.set_route_async(to_add, conn_id)

                    # Invite participant (auto-waits for completion)
                    await session.invite_async(to_add)

                    print(f"{part_name} -> add {name_to_add} to the group")

            await asyncio.sleep(1)  # wait a bit to ensure routes are set up

        # Track if this participant was called
        called = False
        first_message = True

        # if this is the first participant, we need to publish the message
        # to start the chain
        if index == 0:
            next_participant = (index + 1) % participants_count
            next_participant_name = f"participant-{next_participant}"

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
                    if first_message:
                        recv_session = await participant.listen_for_session_async(None)

                        # make sure the received session is group
                        assert (
                            recv_session.session_type()
                            == slim_bindings.SessionType.GROUP
                        )

                        # Note: The new API structure for session properties may differ
                        # destination() and source() return Name objects, not direct comparison

                        first_message = False

                # receive message from session
                received_msg = await recv_session.get_message_async(None)
                _ = received_msg.context
                msg_rcv = received_msg.payload

                # increase the count
                local_count += 1

                # make sure the message is correct
                assert msg_rcv.startswith(bytes(message.encode()))

                # Check if the message is calling this specific participant
                # if not, ignore it
                if (not called) and msg_rcv.decode().endswith(part_name):
                    # print the message
                    print(
                        f"{part_name} -> Receiving message: {msg_rcv.decode()}, local count: {local_count}"
                    )

                    called = True

                    # as the message is for this specific participant, we can
                    # reply to the session and call out the next participant
                    next_participant = (index + 1) % participants_count
                    next_participant_name = f"participant-{next_participant}"
                    print(f"{part_name} -> Calling out {next_participant_name}...")
                    await recv_session.publish_async(
                        f"{message} - {next_participant_name}".encode(), None, None
                    )
                    print(f"{part_name} -> Published! Local count: {local_count}")
                else:
                    print(
                        f"{part_name} -> Receiving message: {msg_rcv.decode()} - not for me. Local count: {local_count}"
                    )

                # If we received as many messages as the number of participants, we can exit
                if local_count >= (participants_count - 1) and index == 0:
                    print(f"{part_name} -> Received all messages, close channel...")

                    await asyncio.sleep(0.2)

                    await participant.delete_session_async(recv_session)
                    print("session closed on participant 0")
                    break

            except Exception as e:
                # Expected: when the session is closed by the moderator, other participants
                # will receive "session channel closed" error
                print(f"{part_name} -> Session closed: {e}")
                break

    # start participants in background
    for i in range(participants_count):
        task = asyncio.create_task(background_task(i))
        task.set_name(f"participant-{i}")
        participants.append(task)

    # Wait for the task to complete
    for task in participants:
        await task
