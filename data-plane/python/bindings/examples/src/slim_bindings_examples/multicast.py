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

    # If provided, split the remote (chat/topic) ID into its components
    chat_topic = split_id(remote) if remote else None

    tasks: list[asyncio.Task] = []

    created_session = None
    if chat_topic and invites:
        format_message_print(
            f"Creating new multicast session (moderator)... {split_id(local)}"
        )
        created_session = await local_app.create_session(
            slim_bindings.PySessionConfiguration.Multicast(
                topic=chat_topic,
                moderator=True,
                max_retries=5,
                timeout=datetime.timedelta(seconds=5),
                mls_enabled=enable_mls,
            )
        )

        # Allow some time before sending invites (mirrors test timing slack)
        await asyncio.sleep(1)

        for invite in invites:
            invite_name = split_id(invite)
            await local_app.set_route(invite_name)
            await created_session.invite(invite_name)
            print(f"{local} -> add {invite_name} to the group")

    async def receive_loop():
        if created_session is None:
            format_message_print(local, "-> Waiting for session...")
            session = await local_app.listen_for_session()
        else:
            session = created_session

        while True:
            try:
                ctx, payload = await session.get_message()
                format_message_print(
                    local,
                    f"-> Received message from {ctx.source_name}: {payload.decode()}",
                )
            except asyncio.CancelledError:
                break
            except Exception as e:
                format_message_print(local, f"-> Error receiving message: {e}")
                break

    tasks.append(asyncio.create_task(receive_loop()))

    if created_session and chat_topic:

        async def keyboard_loop():
            while True:
                user_input = await asyncio.to_thread(input, "\033[1mmessage>\033[0m ")
                if user_input.strip().lower() in ("exit", "quit"):
                    break
                try:
                    await created_session.publish(user_input.encode(), chat_topic)
                except Exception as e:
                    format_message_print(local, f"-> Error sending message: {e}")

        tasks.append(asyncio.create_task(keyboard_loop()))

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
