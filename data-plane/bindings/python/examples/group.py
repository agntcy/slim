# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Group example (heavily commented).

Purpose:
  Demonstrates how to:
    * Start a local Slim app using the global service
    * Optionally create a group session (becoming its moderator)
    * Invite other participants (by their IDs) into the group
    * Receive and display messages
    * Interactively publish messages

Key concepts:
  - Group sessions are created with SessionConfig with SessionType.GROUP and
    reference a 'topic' (channel) Name.
  - Invites are explicit: the moderator invites each participant after
    creating the session.
  - Participants that did not create the session simply wait for
    listen_for_session_async() to yield their Session.

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


async def handle_invite(session, invite_id):
    """Handle inviting a participant to the group."""
    parts = invite_id.split()
    if len(parts) != 1:
        print_formatted_text(
            "Error: 'invite' command expects exactly one participant ID (e.g., 'invite org/ns/client-1')",
            style=custom_style,
        )
        return

    print(f"Inviting participant: {invite_id}")
    invite_name = split_id(invite_id)
    try:
        handle = await session.invite_async(invite_name)
        await handle.wait_async()
    except Exception as e:
        error_str = str(e)
        if "participant already in group" in error_str:
            print_formatted_text(
                f"Error: Participant {invite_id} is already in the group.",
                style=custom_style,
            )
        elif "failed to add participant to session" in error_str:
            print_formatted_text(
                f"Error: Failed to add participant {invite_id} to session.",
                style=custom_style,
            )
        else:
            raise


async def handle_remove(session, remove_id):
    """Handle removing a participant from the group."""
    parts = remove_id.split()
    if len(parts) != 1:
        print_formatted_text(
            "Error: 'remove' command expects exactly one participant ID (e.g., 'remove org/ns/client-1')",
            style=custom_style,
        )
        return

    print(f"Removing participant: {remove_id}")
    remove_name = split_id(remove_id)
    try:
        handle = await session.remove_async(remove_name)
        await handle.wait_async()
    except Exception as e:
        error_str = str(e)
        if "participant not found in group" in error_str:
            print_formatted_text(
                f"Error: Participant {remove_id} is not in the group.",
                style=custom_style,
            )
        else:
            raise


async def receive_loop(
    local_app, created_session, session_ready, shared_session_container
):
    """
    Receive messages for the bound session.

    Behavior:
      * If not moderator: wait for a new group session (listen_for_session_async()).
      * If moderator: reuse the created_session reference.
      * Loop forever until cancellation or an error occurs.
    """
    if created_session is None:
        print_formatted_text("Waiting for session...", style=custom_style)
        session_context = await local_app.listen_for_session_async(None)
        session = session_context
    else:
        session = created_session

    # Make session available to other tasks
    shared_session_container[0] = session
    session_ready.set()

    # Get source and destination names for display
    source_name = session.source()
    dest_name = session.destination()

    while True:
        try:
            # Await next inbound message from the group session.
            # The returned object has .payload (bytes) and .context (MessageContext).
            received_msg = await session.get_message_async(
                timeout=datetime.timedelta(seconds=30)
            )
            ctx = received_msg.context
            payload = received_msg.payload

            # Display sender name and message
            sender = ctx.source_name if hasattr(ctx, "source_name") else source_name
            print_formatted_text(
                f"{sender} > {payload.decode()}",
                style=custom_style,
            )

            # if the message metadata contains PUBLISH_TO this message is a reply
            # to a previous one. In this case we do not reply to avoid loops
            if "PUBLISH_TO" not in ctx.metadata:
                reply = f"message received by {source_name}"
                # Use publish_async instead of publish_to since API changed
                await session.publish_async(reply.encode(), None, ctx.metadata)
        except asyncio.CancelledError:
            # Graceful shutdown path (ctrl-c or program exit).
            break
        except Exception as e:
            # Non-cancellation error; log it.
            print_formatted_text(f"-> Error receiving message: {e}")
            # Break if session is closed, otherwise continue listening
            if "session closed" in str(e).lower():
                break
            continue


async def keyboard_loop(
    created_session, session_ready, shared_session_container, local_app
):
    """
    Interactive loop allowing participants to publish messages.

    Typing 'exit' or 'quit' (case-insensitive) terminates the loop.
    Typing 'remove NAME' removes a participant from the group
    Typing 'invite NAME' invites a participant to the group
    Each line is published to the group channel as UTF-8 bytes.
    """

    try:
        # 1. Initialize an async session
        prompt_session = PromptSession(style=custom_style)

        # Wait for the session to be established
        await session_ready.wait()

        session = shared_session_container[0]
        source_name = session.source()
        dest_name = session.destination()

        if created_session:
            print_formatted_text(
                f"Welcome to the group {dest_name}!\n"
                "Commands:\n"
                "  - Type a message to send it to the group\n"
                "  - 'remove NAME' to remove a participant\n"
                "  - 'invite NAME' to invite a participant\n"
                "  - 'exit' or 'quit' to leave the group",
                style=custom_style,
            )
        else:
            print_formatted_text(
                f"Welcome to the group {dest_name}!\n"
                "Commands:\n"
                "  - Type a message to send it to the group\n"
                "  - 'exit' or 'quit' to leave the group",
                style=custom_style,
            )

        while True:
            # Run blocking input() in a worker thread so we do not block the event loop.
            user_input = await prompt_session.prompt_async(f"{source_name} > ")

            if user_input.lower() in ("exit", "quit"):
                # Delete the session
                handle = await local_app.delete_session_async(
                    shared_session_container[0]
                )
                await handle.wait_async()
                break

            if user_input.lower().startswith("invite "):
                invite_id = user_input[7:].strip()  # Skip "invite " (7 chars)
                await handle_invite(shared_session_container[0], invite_id)
                continue

            if user_input.lower().startswith("remove "):
                remove_id = user_input[7:].strip()  # Skip "remove " (7 chars)
                await handle_remove(shared_session_container[0], remove_id)
                continue

            # Send message to the channel_name specified when creating the session.
            # As the session is group, all participants will receive it.
            await shared_session_container[0].publish_async(
                user_input.encode(), None, None
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
        remote: Channel / topic identity string (org/ns/topic).
        enable_opentelemetry: Activate OTEL tracing if backend available.
        enable_mls: Enable group MLS features.
        shared_secret: Shared secret for symmetric auth (demo only).
        jwt: Path to static JWT token (if using JWT auth).
        spire_trust_bundle: SPIRE trust bundle file path.
        audience: Audience list for JWT verification.
        spire_socket_path: Path to SPIRE agent socket for workload API access.
        spire_target_spiffe_id: Target SPIFFE ID for mTLS authentication with SPIRE.
        spire_jwt_audience: Audience list for SPIRE JWT-SVID validation.
        invites: List of participant IDs to invite (moderator only).
    """
    # Create the local Slim instance using global service
    local_app = await create_local_app(
        local,
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

        # Create group session configuration
        config = slim_bindings.SessionConfig(
            session_type=slim_bindings.SessionType.GROUP,
            enable_mls=enable_mls,
            max_retries=5,
            interval=datetime.timedelta(seconds=5),
            metadata={},
        )

        # Create session - returns a context with completion and session
        session_context = await local_app.create_session_async(config, chat_channel)
        # Wait for session to be established
        await session_context.completion.wait_async()
        created_session = session_context.session

        # Invite each provided participant.
        for invite in invites:
            invite_name = split_id(invite)
            handle = await created_session.invite_async(invite_name)
            await handle.wait_async()
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
            keyboard_loop(
                created_session, session_ready, shared_session_container, local_app
            )
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
