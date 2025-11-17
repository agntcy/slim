# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Group example (heavily commented).

Purpose:
  Demonstrates how to:
    * Start / connect a local Slim app
    * Optionally create a group session (becoming its moderator)
    * Invite other participants (by their IDs) into the group
    * Receive and display messages
    * Interactively publish messages

Key concepts:
  - Group sessions are created with SessionConfiguration.Group and
    reference a 'topic' (channel) Name.
  - Invites are explicit: the moderator invites each participant after
    creating the session.
  - Participants that did not create the session simply wait for
    listen_for_session() to yield their Session.

Usage:
  slim-bindings-examples group \
      --local org/default/me \
      --remote org/default/chat-topic \
      --invites org/default/peer1 --invites org/default/peer2

Notes:
  * If --invites is omitted, the client runs in passive participant mode.
  * If both remote and invites are supplied, the client acts as session moderator.
"""

import asyncio
import datetime

from prompt_toolkit.shortcuts import PromptSession, print_formatted_text
from prompt_toolkit.styles import Style

import slim_bindings

from .common import (
    common_options,
    create_local_app,
    format_message_print,
    split_id,
)

# Prompt style
custom_style = Style.from_dict(
    {
        "system": "ansibrightblue",
        "friend": "ansiyellow",
        "user": "ansigreen",
    }
)


async def receive_loop(
    local_app, created_session, session_ready, shared_session_container
):
    """
    Receive messages for the bound session.

    Behavior:
      * If not moderator: wait for a new group session (listen_for_session()).
      * If moderator: reuse the created_session reference.
      * Loop forever until cancellation or an error occurs.
    """
    if created_session is None:
        print_formatted_text("Waiting for session...", style=custom_style)
        session = await local_app.listen_for_session()
    else:
        session = created_session

    # Make session available to other tasks
    shared_session_container[0] = session
    session_ready.set()

    while True:
        try:
            # Await next inbound message from the group session.
            # The returned parameters are a message context and the raw payload bytes.
            # Check session.py for details on MessageContext contents.
            ctx, payload = await session.get_message()
            print_formatted_text(
                f"{ctx.source_name} > {payload.decode()}",
                style=custom_style,
            )
        except asyncio.CancelledError:
            # Graceful shutdown path (ctrl-c or program exit).
            break
        except Exception as e:
            # Non-cancellation error; surface and exit the loop.
            print_formatted_text(f"-> Error receiving message: {e}")
            break


async def keyboard_loop(session_ready, shared_session_container, local_app):
    """
    Interactive loop allowing participants to publish messages.

    Typing 'exit' or 'quit' (case-insensitive) terminates the loop.
    Each line is published to the group channel as UTF-8 bytes.
    """
    try:
        # 1. Initialize an async session
        prompt_session = PromptSession(style=custom_style)

        # Wait for the session to be established
        await session_ready.wait()

        print_formatted_text(
            f"Welcome to the group {shared_session_container[0].dst}!\nSend a message to the group, or type 'exit' or 'quit' to quit.",
            style=custom_style,
        )

        while True:
            # Run blocking input() in a worker thread so we do not block the event loop.
            user_input = await prompt_session.prompt_async(
                f"{shared_session_container[0].src} > "
            )

            if user_input.lower() in ("exit", "quit"):
                # Also terminate the receive loop.
                handle = await local_app.delete_session(shared_session_container[0])
                await handle
                break

            # Send message to the channel_name specified when creating the session.
            # As the session is group, all participants will receive it.
            await shared_session_container[0].publish(user_input.encode())
    except KeyboardInterrupt:
        # Handle Ctrl+C gracefully
        pass
    except asyncio.CancelledError:
        # Handle task cancellation gracefully
        pass
    except Exception as e:
        print_formatted_text(f"-> Error sending message: {e}")


async def run_client(
    local: str,
    slim: dict,
    remote: str | None,
    enable_opentelemetry: bool = False,
    enable_mls: bool = False,
    shared_secret: str = "secret",
    jwt: str | None = None,
    spire_trust_bundle: str | None = None,
    audience: list[str] | None = None,
    spire_socket_path: str | None = None,
    spire_target_spiffe_id: str | None = None,
    spire_jwt_audience: list[str] | None = None,
    invites: list[str] | None = None,
):
    """
    Orchestrate one group-capable client instance.

    Modes:
      * Moderator (creator): remote (channel) + invites provided.
      * Listener only: no remote; waits for inbound group sessions.

    Args:
        local: Local identity string (org/ns/app).
        slim: Connection config dict (endpoint + tls).
        remote: Channel / topic identity string (org/ns/topic).
        enable_opentelemetry: Activate OTEL tracing if backend available.
        enable_mls: Enable group MLS features.
        shared_secret: Shared secret for symmetric auth (demo only).
        jwt: Path to static JWT token (if using JWT auth).
        spire_trust_bundle: SPIRE trust bundle file path.
        audience: Audience list for JWT verification.
        invites: List of participant IDs to invite (moderator only).
    """
    # Create & connect the local Slim instance (auth derived from args).
    local_app = await create_local_app(
        local,
        slim,
        enable_opentelemetry=enable_opentelemetry,
        shared_secret=shared_secret,
        jwt=jwt,
        spire_trust_bundle=spire_trust_bundle,
        audience=audience,
        spire_socket_path=spire_socket_path,
        spire_target_spiffe_id=spire_target_spiffe_id,
        spire_jwt_audience=spire_jwt_audience,
    )

    # Parse the remote channel/topic if provided; else None triggers passive mode.
    chat_channel = split_id(remote) if remote else None

    # Track background tasks (receiver loop + optional keyboard loop).
    tasks: list[asyncio.Task] = []

    # Session sharing between tasks
    session_ready = asyncio.Event()
    shared_session_container = [None]  # Use list to make it mutable across functions

    # Session object only exists immediately if we are moderator.
    created_session = None
    if chat_channel and invites:
        # We are the moderator; create the group session now.
        format_message_print(
            f"Creating new group session (moderator)... {split_id(local)}"
        )
        config = slim_bindings.SessionConfiguration.Group(
            max_retries=5,  # Max per-message resend attempts upon missing ack before reporting a delivery failure.
            timeout=datetime.timedelta(
                seconds=5
            ),  # Ack / delivery wait window; after this duration a retry is triggered (until max_retries).
            mls_enabled=enable_mls,  # Enable Messaging Layer Security for end-to-end encrypted & authenticated group communication.
        )

        created_session, handle = await local_app.create_session(
            chat_channel,  # Logical group channel (Name) all participants join; acts as group/topic identifier.
            config,  # session configuration
        )

        await handle

        # Invite each provided participant. Route is set before inviting to ensure
        # outbound control messages can reach them. For more info see
        # https://github.com/agntcy/slim/blob/main/data-plane/python/bindings/SESSION.md#invite-a-new-participant
        for invite in invites:
            invite_name = split_id(invite)
            await local_app.set_route(invite_name)
            handle = await created_session.invite(invite_name)
            await handle
            print(f"{local} -> add {invite_name} to the group")

    # Launch the receiver immediately.
    tasks.append(
        asyncio.create_task(
            receive_loop(
                local_app, created_session, session_ready, shared_session_container
            )
        )
    )

    tasks.append(
        asyncio.create_task(
            keyboard_loop(session_ready, shared_session_container, local_app)
        )
    )

    # Wait for any task to finish, then cancel the others.
    try:
        done, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)

        for task in pending:
            task.cancel()

        # We can await the pending tasks to allow them to clean up.
        if pending:
            await asyncio.wait(pending)

        # Raise exceptions from completed tasks, if any
        for task in done:
            exc = task.exception()
            if exc:
                raise exc

    except KeyboardInterrupt:
        # Cancel all tasks on KeyboardInterrupt
        for task in tasks:
            task.cancel()


@common_options
def group_main(
    local: str,
    slim: dict,
    remote: str | None = None,
    enable_opentelemetry: bool = False,
    enable_mls: bool = False,
    shared_secret: str = "secret",
    jwt: str | None = None,
    spire_trust_bundle: str | None = None,
    audience: list[str] | None = None,
    spire_socket_path: str | None = None,
    spire_target_spiffe_id: str | None = None,
    spire_jwt_audience: list[str] | None = None,
    invites: list[str] | None = None,
):
    """
    Synchronous entry-point for the group example (wrapped by Click).

    Converts CLI arguments into a run_client() invocation via asyncio.run().
    """
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
                spire_trust_bundle=spire_trust_bundle,
                audience=audience,
                spire_socket_path=spire_socket_path,
                spire_target_spiffe_id=spire_target_spiffe_id,
                spire_jwt_audience=spire_jwt_audience,
                invites=invites,
            )
        )
    except KeyboardInterrupt:
        print("Client interrupted by user.")
