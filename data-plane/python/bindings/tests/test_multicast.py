# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""
Multicast integration test for Slim bindings.

Scenario:
  - A configurable number of participants (participants_count) join a multicast
    "chat" identified by a shared topic (PyName).
  - Participant 0 (the "moderator") creates the multicast session, invites
    every other participant, and publishes the first message addressed to the
    next participant in a logical ring.
  - Each participant, upon receiving a message that ends with its own name,
    publishes a new message naming the next participant, continuing the ring.
  - Each participant exits after observing (participants_count - 1) messages.

What is validated implicitly:
  * Multicast session establishment (session_type == MULTICAST).
  * dst equals the channel/topic PyName for non-creator participants.
  * src matches the participant's own local identity when receiving.
  * Message propagation across all participants without loss.
  * Optional MLS flag is parameterized.

Note:
  The test relies on timing (sleep calls) to allow route propagation and
  invitation distribution; in a production-grade test suite you might
  replace these with explicit synchronization primitives or polling.
"""

import asyncio
import datetime

import pytest
from common import create_slim

import slim_bindings


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12375"], indirect=True)
@pytest.mark.parametrize("mls_enabled", [False, True])
async def test_multicast(server, mls_enabled):  # noqa: C901
    """Exercise multicast session behavior with N participants relaying a message in a ring.

    Steps:
      1. Participant 0 creates multicast session (optionally with MLS enabled).
      2. Participant 0 invites remaining participants after short delay.
      3. Participant 0 seeds first message naming participant-1.
      4. Each participant that is "called" republishes naming the next participant.
      5. Each participant terminates after seeing (N - 1) messages.

    Args:
        server: Injected server fixture providing an active Slim server endpoint.
        mls_enabled (bool): Whether MLS secure group features are enabled for the session.

    This test asserts invariants via inline assertions and stops when all
    participant tasks complete successfully.
    """
    message = "Calling app"

    # participant count
    participants_count = 10
    participants = []

    chat_name = slim_bindings.PyName("org", "default", "chat")

    # Background task: each participant joins/creates the session and relays messages around the ring
    async def background_task(index):
        """Participant coroutine.

        Responsibilities:
          * Index 0: create multicast session, invite others, publish initial message.
          * Others: wait for inbound session, validate session properties, relay when addressed.
        """
        part_name = f"participant-{index}"
        local_count = 0

        print(f"Creating participant {part_name}...")

        name = slim_bindings.PyName("org", "default", part_name)

        participant = await create_slim(name, "secret")

        # Connect to SLIM server
        _ = await participant.connect(
            {"endpoint": "http://127.0.0.1:12375", "tls": {"insecure": True}}
        )

        if index == 0:
            print(f"{part_name} -> Creating new multicast sessions...")
            # create a multicast session. index 0 is the moderator of the session
            # and it will invite all the other participants to the session
            session = await participant.create_session(
                slim_bindings.PySessionConfiguration.Multicast(
                    channel_name=chat_name,
                    max_retries=5,
                    timeout=datetime.timedelta(seconds=5),
                    mls_enabled=mls_enabled,
                )
            )

            await asyncio.sleep(3)

            # invite all participants
            for i in range(participants_count):
                if i != 0:
                    name_to_add = f"participant-{i}"
                    to_add = slim_bindings.PyName("org", "default", name_to_add)
                    await participant.set_route(to_add)
                    await session.invite(to_add)
                    print(f"{part_name} -> add {name_to_add} to the group")

        # wait a bit for all chat participants to be ready
        await asyncio.sleep(5)

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

            await session.publish(f"{msg}".encode())

        while True:
            try:
                # init session from session
                if index == 0:
                    recv_session = session
                else:
                    if first_message:
                        recv_session = await participant.listen_for_session()

                        # make sure the received session is unicast
                        assert (
                            recv_session.session_type
                            == slim_bindings.PySessionType.MULTICAST
                        )

                        # Make sure the dst of the session is the channel name
                        assert recv_session.dst == chat_name

                        # Make sure the first 3 components of the source belong to participant 0
                        assert recv_session.src == participant.local_name

                        first_message = False

                # receive message from session
                msg_ctx, msg_rcv = await recv_session.get_message()

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

                    # wait a moment to simulate processing time
                    await asyncio.sleep(0.1)

                    # as the message is for this specific participant, we can
                    # reply to the session and call out the next participant
                    next_participant = (index + 1) % participants_count
                    next_participant_name = f"participant-{next_participant}"
                    print(f"{part_name} -> Calling out {next_participant_name}...")
                    await recv_session.publish(
                        f"{message} - {next_participant_name}".encode()
                    )
                else:
                    print(
                        f"{part_name} -> Receiving message: {msg_rcv.decode()} - not for me. Local count: {local_count}"
                    )

                # If we received as many messages as the number of participants, we can exit
                if local_count >= (participants_count - 1):
                    print(f"{part_name} -> Received all messages, exiting...")
                    await participant.delete_session(recv_session)
                    break

            except Exception as e:
                print(f"{part_name} -> Error receiving message: {e}")
                break

    # start participants in background
    for i in range(participants_count):
        task = asyncio.create_task(background_task(i))
        task.set_name(f"participant-{i}")
        participants.append(task)
        await asyncio.sleep(0.1)

    # Wait for the task to complete
    for task in participants:
        await task
