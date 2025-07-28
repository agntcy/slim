# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime

import click

import slim_bindings

from .common import (
    common_options,
    create_local_app,
    format_message,
    split_id,
)


async def run_client(
    local: str,
    slim: dict,
    remote: str | None,
    enable_opentelemetry: bool = False,
    shared_secret: str | None = None,
    jwt: str | None = None,
    bundle: str | None = None,
    audience: list[str] | None = None,
    message: str | None = None,
    iterations: int = 10,
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

    instance = local_app.get_agent_id()

    async with local_app:
        if message:
            # Create a route to the remote ID
            await local_app.set_route(
                remote_organization, remote_namespace, broadcast_topic
            )

            # create streaming session with default config
            session_info = await local_app.create_session(
                slim_bindings.PySessionConfiguration.Streaming(
                    slim_bindings.PySessionDirection.SENDER,
                    topic=None,
                    max_retries=5,
                    timeout=datetime.timedelta(seconds=5),
                )
            )

            # initialize count
            count = 0

            for _ in range(iterations):
                try:
                    # Send a message
                    print(
                        format_message(
                            f"{instance} streaming message to {remote_organization}/{remote_namespace}/{broadcast_topic}: ",
                            f"{count}",
                        )
                    )

                    # Send the message and wait for a reply
                    await local_app.publish(
                        session_info,
                        f"{count}".encode(),
                        remote_organization,
                        remote_namespace,
                        broadcast_topic,
                    )

                    count += 1
                except Exception as e:
                    print("received error: ", e)
                finally:
                    await asyncio.sleep(0.5)
        else:
            # subscribe to streaming session
            await local_app.subscribe(
                remote_organization, remote_namespace, broadcast_topic
            )

            # Wait for messages and not reply
            while True:
                session_info, _ = await local_app.receive()
                print(
                    format_message(
                        f"{instance} received a new session:",
                        f"{session_info.id}",
                    )
                )

                async def background_task(session_info):
                    while True:
                        # Receive the message from the session
                        session, msg = await local_app.receive(session=session_info)
                        print(
                            format_message(
                                f"{instance} received from {remote_organization}/{remote_namespace}/{broadcast_topic}: ",
                                f"{msg.decode()}",
                            )
                        )

                asyncio.create_task(background_task(session_info.id))


def streaming_options(function):
    function = click.option(
        "--message",
        type=str,
        help="Message to send.",
    )(function)

    function = click.option(
        "--iterations",
        type=int,
        default=10,
        help="Number of iterations to send messages.",
    )(function)

    return function


@common_options
@streaming_options
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
):
    try:
        asyncio.run(
            run_client(
                local=local,
                slim=slim,
                remote=remote,
                enable_opentelemetry=enable_opentelemetry,
                shared_secret=shared_secret,
                jwt=jwt,
                bundle=bundle,
                audience=audience,
                message=message,
                iterations=iterations,
            )
        )
    except KeyboardInterrupt:
        print("Client interrupted by user.")
