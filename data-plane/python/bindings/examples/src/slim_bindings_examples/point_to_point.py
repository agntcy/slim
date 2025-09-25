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
    unicast: bool = False,
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

    instance = local_app.id

    if message:
        if not remote:
            raise ValueError("Remote ID must be provided when message is specified.")

    # Wrapper objects are not async context managers; operate directly.
    if message:
        remote_name = split_id(remote)
        await local_app.set_route(remote_name)

        # Select session type. Unicast enables sticky semantics; Anycast for fan-out.
        if unicast or enable_mls:
            session = await local_app.create_session(
                slim_bindings.PySessionConfiguration.Unicast(  # type: ignore
                    unicast_name=remote_name,
                    max_retries=5,
                    timeout=datetime.timedelta(seconds=5),
                    mls_enabled=enable_mls,
                ),
            )
        else:
            session = await local_app.create_session(
                slim_bindings.PySessionConfiguration.Anycast()  # type: ignore
            )

        for i in range(iterations):
            try:
                if unicast or enable_mls:
                    await session.publish(message.encode())
                else:
                    await session.publish_with_destination(
                        message.encode(), remote_name
                    )
                format_message_print(
                    f"{instance}",
                    f"Sent message {message} - {i + 1}/{iterations}:",
                )
                _msg_ctx, reply = await session.get_message()
                format_message_print(
                    f"{instance}",
                    f"received (from session {session.id}): {reply.decode()}",
                )
            except Exception as e:
                format_message_print(f"{instance}", f"error: {e}")
            await asyncio.sleep(1)
    else:
        while True:
            format_message_print(
                f"{instance}", "waiting for new session to be established"
            )
            session = await local_app.listen_for_session()
            format_message_print(f"{instance}", f"new session {session.id}")

            async def session_loop(sess: slim_bindings.PySession):  # type: ignore
                while True:
                    try:
                        msg_ctx, payload = await sess.get_message()
                    except Exception:
                        # session probably closed
                        break
                    text = payload.decode()
                    format_message_print(f"{instance}", f"received: {text}")
                    await sess.publish_to(msg_ctx, f"{text} from {instance}".encode())

            asyncio.create_task(session_loop(session))


def p2p_options(function):
    function = click.option(
        "--message",
        type=str,
        help="Message to send.",
    )(function)

    function = click.option(
        "--iterations",
        type=int,
        help="Number of messages to send, one per second.",
        default=10,
    )(function)

    return function


@common_options
@p2p_options
def main_anycast(
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
                enable_mls=False,
                shared_secret=shared_secret,
                jwt=jwt,
                bundle=bundle,
                audience=audience,
                message=message,
                iterations=iterations,
                unicast=False,
            )
        )
    except KeyboardInterrupt:
        print("Client interrupted by user.")


@common_options
@p2p_options
def main_unicast(
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
                enable_mls=enable_mls,
                shared_secret=shared_secret,
                jwt=jwt,
                bundle=bundle,
                audience=audience,
                message=message,
                iterations=iterations,
                unicast=True,
            )
        )
    except KeyboardInterrupt:
        print("Client interrupted by user.")
