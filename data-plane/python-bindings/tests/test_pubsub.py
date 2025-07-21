# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime

import pytest
from common import create_slim

import slim_bindings


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12375"], indirect=True)
@pytest.mark.parametrize("mls_enabled", [True, False])
async def test_pubsub(server, mls_enabled):  # noqa: C901
    org = "cisco"
    ns = "default"
    chat = "chat"

    message = "Calling agent"

    # participant count
    participants_count = 10
    participants = []

    # define the background task
    async def background_task(index):
        name = f"participant-{index}"
        local_count = 0

        print(f"Creating participant {name}...")

        participant = await create_slim(org, ns, name, "secret")

        # Connect to SLIM server
        _ = await participant.connect(
            {"endpoint": "http://127.0.0.1:12375", "tls": {"insecure": True}}
        )

        if index == 0:
            print(f"{name} -> Creating new pubsub sessions...")
            # create pubsubb session. index 0 is the moderator of the session
            # and it will invite all the other participants to the session
            session_info = await participant.create_session(
                slim_bindings.PySessionConfiguration.Streaming(
                    slim_bindings.PySessionDirection.BIDIRECTIONAL,
                    topic=slim_bindings.PyAgentType(org, ns, chat),
                    moderator=True,
                    max_retries=5,
                    timeout=datetime.timedelta(seconds=5),
                    mls_enabled=mls_enabled,
                )
            )

            await asyncio.sleep(3)

            # invite all participants
            for i in range(participants_count):
                if i != 0:
                    type_to_add = f"participant-{i}"
                    to_add = slim_bindings.PyAgentType(org, ns, type_to_add)
                    await participant.set_route(org, ns, type_to_add)
                    await participant.invite(session_info, to_add)
                    print(f"{name} -> add {type_to_add} to the group")

        # wait a bit for all chat participants to be ready
        await asyncio.sleep(5)

        # Track if this participant was called
        called = False
        first_message = True

        async with participant:
            # if this is the first participant, we need to publish the message
            # to start the chain
            if index == 0:
                next_participant = (index + 1) % participants_count
                next_participant_name = f"participant-{next_participant}"

                msg = f"{message} - {next_participant_name}"

                print(f"{name} -> Publishing message as first participant: {msg}")

                called = True

                await participant.publish(
                    session_info,
                    f"{msg}".encode(),
                    org,
                    ns,
                    chat,
                )

            while True:
                try:
                    # init session from session
                    if index == 0:
                        recv_session = session_info
                    else:
                        if first_message:
                            recv_session, _ = await participant.receive()
                            first_message = False

                    # receive message from session
                    recv_session, msg_rcv = await participant.receive(
                        session=recv_session.id
                    )

                    # increase the count
                    local_count += 1

                    # make sure the message is correct
                    assert msg_rcv.startswith(bytes(message.encode()))

                    # Check if the message is calling this specific participant
                    # if not, ignore it
                    if (not called) and msg_rcv.decode().endswith(name):
                        # print the message
                        print(
                            f"{name} -> Receiving message: {msg_rcv.decode()}, local count: {local_count}"
                        )

                        called = True

                        # wait a moment to simulate processing time
                        await asyncio.sleep(0.1)

                        # as the message is for this specific participant, we can
                        # reply to the session and call out the next participant
                        next_participant = (index + 1) % participants_count
                        next_participant_name = f"participant-{next_participant}"
                        print(f"{name} -> Calling out {next_participant_name}...")
                        await participant.publish(
                            recv_session,
                            f"{message} - {next_participant_name}".encode(),
                            org,
                            ns,
                            chat,
                        )
                    else:
                        print(
                            f"{name} -> Receiving message: {msg_rcv.decode()} - not for me. Local count: {local_count}"
                        )

                    # If we received as many messages as the number of participants, we can exit
                    if local_count >= (participants_count - 1):
                        print(f"{name} -> Received all messages, exiting...")
                        await participant.delete_session(recv_session.id)
                        break

                except Exception as e:
                    print(f"{name} -> Error receiving message: {e}")
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
