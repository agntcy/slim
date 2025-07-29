# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime

import click

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
    message: str | None = None,
    iterations: int = 1,
    sticky: bool = False,
):
    local_app: slim_bindings.Slim = await create_local_app(
        local,
        slim,
        enable_opentelemetry=enable_opentelemetry,
        shared_secret=shared_secret,
        jwt=jwt,
        bundle=bundle,
        audience=audience,
    )

    instance = local_app.get_agent_id()

    if message:
        if not remote:
            raise ValueError("Remote ID must be provided when message is specified.")

    async with local_app:
        if message:
            # Split the IDs into their respective components
            remote_organization, remote_namespace, remote_agent = split_id(remote)

            # Create a route to the remote ID
            await local_app.set_route(
                remote_organization, remote_namespace, remote_agent
            )

            # create a session
            if sticky or enable_mls:
                session = await local_app.create_session(
                    slim_bindings.PySessionConfiguration.FireAndForget(
                        max_retries=5,
                        timeout=datetime.timedelta(seconds=5),
                        sticky=True,
                        mls_enabled=enable_mls,
                    )
                )
            else:
                session = await local_app.create_session(
                    slim_bindings.PySessionConfiguration.FireAndForget()
                )

            for i in range(0, iterations):
                try:
                    # Send the message
                    await local_app.publish(
                        session,
                        message.encode(),
                        remote_organization,
                        remote_namespace,
                        remote_agent,
                    )

                    format_message_print(
                        f"{instance}",
                        f"Sent message {message} - {i + 1}/{iterations}:",
                    )

                    # Wait for a reply
                    session_info, msg = await local_app.receive(session=session.id)

                    format_message_print(
                        f"{instance}",
                        f"received (from session {session_info.id}): {msg.decode()}",
                    )

                except Exception as e:
                    print("received error: ", e)

                await asyncio.sleep(1)
        else:
            # Wait for a message and reply in a loop
            while True:
                format_message_print(
                    f"{instance}",
                    "waiting for new session to be established",
                )

                session_info, _ = await local_app.receive()
                format_message_print(
                    f"{instance} received a new session:",
                    f"{session_info.id}",
                )

                async def background_task(session_id):
                    while True:
                        # Receive the message from the session
                        session, msg = await local_app.receive(session=session_id)
                        format_message_print(
                            f"{instance}",
                            f"received (from session {session_id}): {msg.decode()}",
                        )

                        ret = f"{msg.decode()} from {instance}"

                        await local_app.publish_to(session, ret.encode())
                        format_message_print(f"{instance}", f"replies: {ret}")

                asyncio.create_task(background_task(session_info.id))


def ff_options(function):
    function = click.option(
        "--message",
        type=str,
        help="Message to send.",
    )(function)

    function = click.option(
        "--iterations",
        type=int,
        help="Number of messages to send, one per second.",
        default=2,
    )(function)

    function = click.option(
        "--sticky",
        is_flag=True,
        help="Enable FF sessions to connect always to the same endpoint.",
        default=False,
    )(function)

    return function


@common_options
@ff_options
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
    message: str | None = None,
    iterations: int = 1,
    sticky: bool = False,
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
                message=message,
                iterations=iterations,
                sticky=sticky,
            )
        )
    except KeyboardInterrupt:
        print("Client interrupted by user.")
