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
        broadcast_topic = split_id(remote)

    tasks = []

    session_info = None
    if remote and invites:
        format_message_print(local, "Creating new multicast sessions...")
        session_info = await local_app.create_session(
            slim_bindings.PySessionConfiguration.Multicast(  # type: ignore
                topic=broadcast_topic,
                moderator=True,
                max_retries=5,
                timeout=datetime.timedelta(seconds=5),
                mls_enabled=enable_mls,
            )
        )
        for p in invites:
            to_add = split_id(p)
            await local_app.set_route(to_add)
            await session_info.invite(to_add)
            print(f"{local} -> add {to_add} to the group")

    # define the background task
    async def background_task():
        if session_info is None:
            format_message_print(local, "-> Waiting for session...")
            recv_session = await local_app.listen_for_session()
        else:
            recv_session = session_info
        while True:
            try:
                _ctx, msg_rcv = await recv_session.get_message()
                format_message_print(
                    local,
                    f"-> Received message from {recv_session.id}: {msg_rcv.decode()}",
                )
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
                await session_info.publish(f"{user_input}".encode(), broadcast_topic)

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
