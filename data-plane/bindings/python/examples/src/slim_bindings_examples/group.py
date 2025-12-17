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
  slim-uniffi-examples group \
      --local org/default/me \
      --remote org/default/chat-topic \
      --invites org/default/peer1 --invites org/default/peer2

Notes:
  * If --invites is omitted, the client runs in passive participant mode.
  * If both remote and invites are supplied, the client acts as session moderator.
"""

import asyncio

from prompt_toolkit.shortcuts import PromptSession, print_formatted_text
from prompt_toolkit.styles import Style

import slim_uniffi_bindings._slim_bindings.slim_bindings as slim

from .common import (
    common_options,
    create_local_app,
    create_session_config,
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
    local_app, conn_id, created_session, session_ready, shared_session_container
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
        session = await local_app.listen_for_session_async(timeout_ms=None)
    else:
        session = created_session

    # Make session available to other tasks
    shared_session_container[0] = session
    session_ready.set()

    while True:
        try:
            # Await next inbound message from the group session.
            # The returned object has context and payload fields.
            msg = await session.get_message_async(timeout_ms=None)
            source_name_components = msg.context.source_name.components
            source_name = "/".join(source_name_components)
            print_formatted_text(
                f"{source_name} > {msg.payload.decode()}",
                style=custom_style,
            )
            # if the message metadata contains PUBLISH_TO this message is a reply
            # to a previous one. In this case we do not reply to avoid loops
            if "PUBLISH_TO" not in msg.context.metadata:
                reply = f"message received by {session.source().id}"
                await session.publish_to_async(
                    msg.context, reply.encode(), "text/plain", {}
                )
        except asyncio.CancelledError:
            # Graceful shutdown path (ctrl-c or program exit).
            break
        except Exception as e:
            # Non-cancellation error; log it.
            print_formatted_text(f"-> Error receiving message: {e}")
            # Break if session is closed, otherwise continue listening
            if "session channel closed" in str(e).lower():
                break
            continue


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

        session = shared_session_container[0]
        dest_components = session.destination().components
        dest_name = "/".join(dest_components)
        src_id = session.source().id

        print_formatted_text(
            f"Welcome to the group {dest_name}!\nSend a message to the group, or type 'exit' or 'quit' to quit.",
            style=custom_style,
        )

        while True:
            # Run blocking input() in a worker thread so we do not block the event loop.
            user_input = await prompt_session.prompt_async(f"{src_id} > ")

            if user_input.lower() in ("exit", "quit"):
                # Also terminate the receive loop.
                await local_app.delete_session_async(shared_session_container[0])
                break

            # Send message to the channel_name specified when creating the session.
            # As the session is group, all participants will receive it.
            await shared_session_container[0].publish_async(
                user_input.encode(), "text/plain", {}
            )
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
    slim_config: dict,
    remote: str | None,
    enable_opentelemetry: bool = False,
    enable_mls: bool = False,
    shared_secret: str = "secret",
    invites: list[str] | None = None,
):
    """
    Orchestrate one group-capable client instance.

    Modes:
      * Moderator (creator): remote (channel) + invites provided.
      * Listener only: no remote; waits for inbound group sessions.

    Args:
        local: Local identity string (org/ns/app).
        slim_config: Connection config dict (endpoint + tls).
        remote: Channel / topic identity string (org/ns/topic).
        enable_opentelemetry: Activate OTEL tracing if backend available.
        enable_mls: Enable group MLS features.
        shared_secret: Shared secret for symmetric auth (demo only).
        invites: List of participant IDs to invite (moderator only).
    """
    # Create & connect the local Slim instance (auth derived from args).
    local_app, conn_id = await create_local_app(
        local,
        slim_config,
        enable_opentelemetry=enable_opentelemetry,
        shared_secret=shared_secret,
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
            f"Creating new group session (moderator)... {split_id(local).id}"
        )
        config = create_session_config(
            session_type="Group",
            enable_mls=enable_mls,
            max_retries=5,
            interval_ms=100,
            initiator=True,
        )

        created_session = await local_app.create_session_async(
            config,
            chat_channel,
        )

        # Invite each provided participant.
        for invite in invites:
            invite_name = split_id(invite)
            await local_app.set_route_async(invite_name, conn_id)
            await created_session.invite_async(invite_name)
            print(f"{local} -> add {invite_name.id} to the group")

    # Launch the receiver immediately.
    tasks.append(
        asyncio.create_task(
            receive_loop(
                local_app, conn_id, created_session, session_ready, shared_session_container
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
                slim_config=slim,
                remote=remote,
                enable_opentelemetry=enable_opentelemetry,
                enable_mls=enable_mls,
                shared_secret=shared_secret,
                invites=invites,
            )
        )
    except KeyboardInterrupt:
        print("Client interrupted by user.")
