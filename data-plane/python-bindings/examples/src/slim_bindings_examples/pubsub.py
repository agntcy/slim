# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime

import slim_bindings

from .common import (
    common_options,
    create_local_app,
    format_message_print,
    split_id,
)


async def run_client(
    local: str,
    slim: dict,
    remote: str | None,
    enable_opentelemetry: bool = False,
    enable_mls: bool = False,
    shared_secret: str | None = None,
    jwt: str | None = None,
    bundle: str | None = None,
    audience: list[str] | None = None,
    invites: list[str] | None = None,
):
    local_app = await create_local_app(
        local,
        slim,
        enable_opentelemetry=enable_opentelemetry,
        shared_secret=shared_secret,
        jwt=jwt,
        bundle=bundle,
        audience=audience,
    )

    # If provided, split the remote IDs into their respective components
    if remote:
        remote_organization, remote_namespace, broadcast_topic = split_id(remote)

    tasks = []

    if remote and invites:
        format_message_print(local, "Creating new pubsub sessions...")
        # create pubsubb session. A pubsub session is a just a bidirectional
        # streaming session, where participants are both sender and receivers
        session_info = await local_app.create_session(
            slim_bindings.PySessionConfiguration.Streaming(
                slim_bindings.PySessionDirection.BIDIRECTIONAL,
                topic=slim_bindings.PyAgentType(
                    remote_organization, remote_namespace, broadcast_topic
                ),
                moderator=True,
                max_retries=5,
                timeout=datetime.timedelta(seconds=5),
                mls_enabled=enable_mls,
            )
        )

        # invite all participants
        for p in invites:
            org, ns, type_to_add = split_id(p)
            to_add = slim_bindings.PyAgentType(org, ns, type_to_add)
            await local_app.set_route(org, ns, type_to_add)
            await local_app.invite(session_info, to_add)
            print(f"{local} -> add {type_to_add} to the group")

    # define the background task
    async def background_task():
        async with local_app:
            # init session from session
            if invites:
                recv_session = session_info
            else:
                format_message_print(local, "-> Waiting for session...")
                recv_session, _ = await local_app.receive()

            while True:
                try:
                    # receive message from session
                    recv_session, msg_rcv = await local_app.receive(
                        session=recv_session.id
                    )

                    # print received message
                    format_message_print(
                        local,
                        f"-> Received message from {recv_session.id}: {msg_rcv.decode()}",
                    )

                    # Here we could send a message back to the channel, e.g.:
                    # await participant.publish(session_info, f"Echo: {msg_rcv.decode()}".encode())
                except asyncio.CancelledError:
                    break
                except Exception as e:
                    format_message_print(local, f"-> Error receiving message: {e}")
                    break

    tasks.append(asyncio.create_task(background_task()))

    if remote and invites:

        async def background_task_keyboard():
            while True:
                user_input = await asyncio.to_thread(input, "\033[1mmessage>\033[0m ")
                if user_input == "exit":
                    break

                # Send the message to the all participants
                await local_app.publish(
                    session_info,
                    f"{user_input}".encode(),
                    remote_organization,
                    remote_namespace,
                    broadcast_topic,
                )

        tasks.append(asyncio.create_task(background_task_keyboard()))

    # Wait for both tasks to finish
    await asyncio.gather(*tasks)


@common_options
def main(
    local: str,
    slim: dict,
    remote: str | None = None,
    enable_opentelemetry: bool = False,
    enable_mls: bool = False,
    shared_secret: str | None = None,
    jwt: str | None = None,
    bundle: str | None = None,
    audience: list[str] | None = None,
    invites: list[str] | None = None,
):
    try:
        asyncio.run(
            run_client(
                local=local,
                slim=slim,
                remote=remote,
                enable_opentelemetry=enable_opentelemetry,
                enable_mls=enable_mls,
                shared_secret=shared_secret,
                jwt=jwt,
                bundle=bundle,
                audience=audience,
                invites=invites,
            )
        )
    except KeyboardInterrupt:
        print("Client interrupted by user.")
