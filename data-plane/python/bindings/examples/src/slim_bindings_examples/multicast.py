# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Multicast example (heavily commented).

Purpose:
  Demonstrates how to:
    * Start / connect a local Slim app
    * Optionally create a multicast session (becoming its moderator)
    * Invite other participants (by their IDs) into the multicast group
    * Receive and display messages
    * Interactively publish messages (moderator only)

Key concepts:
  - Multicast sessions are created with PySessionConfiguration.Multicast and
    reference a 'topic' (channel) PyName.
  - Invites are explicit: the moderator invites each participant after
    creating the session.
  - Participants that did not create the session simply wait for
    listen_for_session() to yield their PySession.

Usage:
  slim-bindings-examples multicast \
      --local org/default/me \
      --remote org/default/chat-topic \
      --invites org/default/peer1 --invites org/default/peer2

Notes:
  * If --invites is omitted, the client runs in passive participant mode.
  * If both remote and invites are supplied, the client acts as session moderator.
"""

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
    shared_secret: str = "secret",
    jwt: str | None = None,
    spire_trust_bundle: str | None = None,
    audience: list[str] | None = None,
    invites: list[str] | None = None,
):
    """
    Orchestrate one multicast-capable client instance.

    Modes:
      * Moderator (creator): remote (channel) + invites provided.
      * Listener only: no remote; waits for inbound multicast sessions.

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
    )

    # Parse the remote channel/topic if provided; else None triggers passive mode.
    chat_channel = split_id(remote) if remote else None

    # Track background tasks (receiver loop + optional keyboard loop).
    tasks: list[asyncio.Task] = []

    # Session object only exists immediately if we are moderator.
    created_session = None
    if chat_channel and invites:
        # We are the moderator; create the multicast session now.
        format_message_print(
            f"Creating new multicast session (moderator)... {split_id(local)}"
        )
        created_session = await local_app.create_session(
            slim_bindings.PySessionConfiguration.Multicast(  # type: ignore  # Build multicast session configuration
                channel_name=chat_channel,  # Logical multicast channel (PyName) all participants join; acts as group/topic identifier.
                max_retries=5,  # Max per-message resend attempts upon missing ack before reporting a delivery failure.
                timeout=datetime.timedelta(
                    seconds=5
                ),  # Ack / delivery wait window; after this duration a retry is triggered (until max_retries).
                mls_enabled=enable_mls,  # Enable Messaging Layer Security for end-to-end encrypted & authenticated group communication.
            )
        )

        # Small delay so underlying routing / session creation stabilizes.
        await asyncio.sleep(1)

        # Invite each provided participant. Route is set before inviting to ensure
        # outbound control messages can reach them.
        for invite in invites:
            invite_name = split_id(invite)
            await local_app.set_route(invite_name)
            await created_session.invite(invite_name)
            print(f"{local} -> add {invite_name} to the group")

    async def receive_loop():
        """
        Receive messages for the bound session.

        Behavior:
          * If not moderator: wait for a new multicast session (listen_for_session()).
          * If moderator: reuse the created_session reference.
          * Loop forever until cancellation or an error occurs.
        """
        if created_session is None:
            format_message_print(local, "-> Waiting for session...")
            session = await local_app.listen_for_session()
        else:
            session = created_session

        while True:
            try:
                # Await next inbound message from the multicast session.
                # The returned parameters are a message context and the raw payload bytes.
                # Check session.py for details on PyMessageContext contents.
                ctx, payload = await session.get_message()
                format_message_print(
                    local,
                    f"-> Received message from {ctx.source_name}: {payload.decode()}",
                )
            except asyncio.CancelledError:
                # Graceful shutdown path (ctrl-c or program exit).
                break
            except Exception as e:
                # Non-cancellation error; surface and exit the loop.
                format_message_print(local, f"-> Error receiving message: {e}")
                break

    # Launch the receiver immediately (moderator or participant).
    tasks.append(asyncio.create_task(receive_loop()))

    # Only moderators with a known chat_topic get an interactive publishing loop.
    if created_session and chat_channel:

        async def keyboard_loop():
            """
            Interactive loop allowing moderator to publish messages.

            Typing 'exit' or 'quit' (case-insensitive) terminates the loop.
            Each line is published to the multicast topic as UTF-8 bytes.
            """
            while True:
                # Run blocking input() in a worker thread so we do not block the event loop.
                user_input = await asyncio.to_thread(input, "\033[1mmessage>\033[0m ")
                if user_input.strip().lower() in ("exit", "quit"):
                    break
                try:
                    # Send message to the channel_name specified when creating the session.
                    # As the session is multicast, all participants will receive it.
                    # calling publish_with_destination on a multicast session will raise an error.
                    await created_session.publish(user_input.encode())
                except Exception as e:
                    format_message_print(local, f"-> Error sending message: {e}")

        tasks.append(asyncio.create_task(keyboard_loop()))

    # Wait for all spawned tasks. In moderator mode, this includes keyboard loop.
    await asyncio.gather(*tasks)


@common_options
def main(
    local: str,
    slim: dict,
    remote: str | None = None,
    enable_opentelemetry: bool = False,
    enable_mls: bool = False,
    shared_secret: str = "secret",
    jwt: str | None = None,
    spire_trust_bundle: str | None = None,
    audience: list[str] | None = None,
    invites: list[str] | None = None,
):
    """
    Synchronous entry-point for the multicast example (wrapped by Click).

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
                invites=invites,
            )
        )
    except KeyboardInterrupt:
        print("Client interrupted by user.")
