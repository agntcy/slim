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
  slim-bindings-group \
      --local org/default/me \
      --remote org/default/chat-topic \
      --invites org/default/peer1 --invites org/default/peer2

Notes:
  * If --invites is omitted, the client runs in passive participant mode.
  * If both remote and invites are supplied, the client acts as session moderator.
"""

import asyncio
import datetime
import sys

from prompt_toolkit.shortcuts import PromptSession, print_formatted_text
from prompt_toolkit.styles import Style

import slim_bindings

from .common import (
    create_base_parser,
    create_local_app,
    format_message_print,
    parse_args_to_dict,
    split_id,
)
from .config import GroupConfig, load_config_with_cli_override

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


async def handle_remove(session: slim_bindings.Session, remove_id: str):
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
    local_app: slim_bindings.App,
    created_session: slim_bindings.Session | None,
    session_ready: asyncio.Event,
    shared_session_container: list,
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
        session = await local_app.listen_for_session_async(None)
    else:
        session = created_session

    # Make session available to other tasks
    shared_session_container[0] = session
    session_ready.set()

    # Get source and destination names for display
    source_name = session.source()

    while True:
        try:
            # Await next inbound message from the group session.
            # Returns tuple (MessageContext, bytes).
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
                await session.publish_to_async(ctx, reply.encode(), None, ctx.metadata)
        except asyncio.CancelledError:
            # Graceful shutdown path (ctrl-c or program exit).
            break
        except Exception as e:
            # Break if session is closed, otherwise continue listening
            if "session closed" in str(e).lower():
                break
            continue


async def keyboard_loop(
    created_session: slim_bindings.Session,
    session_ready: asyncio.Event,
    shared_session_container: list[slim_bindings.Session],
    local_app: slim_bindings.App,
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

            if user_input.lower() in ("exit", "quit") and created_session:
                # Delete the session
                handle = await local_app.delete_session_async(
                    shared_session_container[0]
                )
                await handle.wait_async()
                break

            if user_input.lower().startswith("invite ") and created_session:
                invite_id = user_input[7:].strip()  # Skip "invite " (7 chars)
                await handle_invite(shared_session_container[0], invite_id)
                continue

            if user_input.lower().startswith("remove ") and created_session:
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


async def run_client(config: GroupConfig):
    """
    Orchestrate one group-capable client instance.

    Modes:
      * Moderator (creator): remote (channel) + invites provided.
      * Listener only: no remote; waits for inbound group sessions.

    Args:
        config: GroupConfig instance containing all configuration.
    """
    # Create the local Slim instance using global service
    local_app, conn_id = await create_local_app(config)

    # Parse the remote channel/topic if provided; else None triggers passive mode.
    chat_channel = split_id(config.remote) if config.remote else None

    # Track background tasks (receiver loop + optional keyboard loop).
    tasks: list[asyncio.Task] = []

    # Session sharing between tasks
    session_ready = asyncio.Event()
    shared_session_container = [None]  # Use list to make it mutable across functions

    # Session object only exists immediately if we are moderator.
    created_session = None
    if chat_channel and config.invites:
        # We are the moderator; create the group session now.
        format_message_print(
            f"Creating new group session (moderator)... {split_id(config.local)}"
        )

        # Create group session configuration
        session_config = slim_bindings.SessionConfig(
            session_type=slim_bindings.SessionType.GROUP,
            enable_mls=config.enable_mls,
            max_retries=5,
            interval=datetime.timedelta(seconds=5),
            metadata={},
        )

        # Create session - returns a tuple (SessionContext, CompletionHandle)
        session = local_app.create_session(session_config, chat_channel)
        # Wait for session to be established
        await session.completion.wait_async()
        created_session = session.session

        # Invite each provided participant.
        for invite in config.invites:
            invite_name = split_id(invite)
            await local_app.set_route_async(invite_name, conn_id)
            handle = await created_session.invite_async(invite_name)
            await handle.wait_async()
            print(f"{config.local} -> add {invite_name} to the group")

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


def main():
    """
    CLI entry-point for the group example.

    Parses command-line arguments and config file, then runs the client.
    """
    # Create parser with common options
    parser = create_base_parser(
        description="SLIM Group Messaging Example\n\n"
        "Create or join a group messaging session with multiple participants."
    )

    # Add group-specific options
    parser.add_argument(
        "--invites",
        type=str,
        action="append",
        dest="invites",
        help="Invite participant to the group session (can be specified multiple times)",
    )

    # Parse arguments
    args = parser.parse_args()

    # Convert to dictionary
    args_dict = parse_args_to_dict(args)

    # Load configuration (CLI args override env vars and config file)
    try:
        config = load_config_with_cli_override(GroupConfig, args_dict)
    except Exception as e:
        print(f"Configuration error: {e}", file=sys.stderr)
        sys.exit(1)

    # Run the client
    try:
        asyncio.run(run_client(config))
    except KeyboardInterrupt:
        print("\nClient interrupted by user.")
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
