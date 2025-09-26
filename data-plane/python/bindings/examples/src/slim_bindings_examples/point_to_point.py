# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Point-to-point messaging example (ANYCAST / UNICAST) for Slim bindings.

This example can operate in two primary modes:

1. Active sender (message mode):
   - Creates a session (ANYCAST or UNICAST depending on flags).
   - Publishes a fixed or user-supplied message multiple times to a remote identity.
   - Receives replies for each sent message (request/reply pattern).

2. Passive listener (no --message provided):
   - Waits for inbound sessions initiated by a remote party.
   - Echoes replies for each received payload, tagging them with the local instance ID.

Key concepts demonstrated:
  - Slim.new() construction and connection.
  - Route establishment (set_route) prior to establishing a session.
  - ANYCAST vs UNICAST session creation logic.
  - Publish / receive loop with per-message reply.
  - Simple flow control via iteration count and sleeps (demo-friendly).

Notes:
  * ANYCAST sessions route to any available peer instance (load-balanced).
  * UNICAST sessions stick to one specific peer (sticky / affinity semantics).
  * MLS flag forces UNICAST mode (since group security semantics differ).

The heavy inline comments are intentional to guide new users line-by-line.
"""

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
    shared_secret: str = "secret",
    jwt: str | None = None,
    spire_trust_bundle: str | None = None,
    audience: list[str] | None = None,
    message: str | None = None,
    iterations: int = 1,
    unicast: bool = False,
):
    """
    Core coroutine that performs either active send or passive listen logic.

    Args:
        local: Local identity string (org/namespace/app).
        slim: Connection configuration dict (endpoint, tls, etc.).
        remote: Remote identity target for routing / session initiation.
        enable_opentelemetry: Enable OpenTelemetry tracing if configured.
        enable_mls: Enable MLS (forces UNICAST for point-to-point).
        shared_secret: Shared secret for symmetric auth (dev/demo).
        jwt: Path to static JWT token (if JWT auth path chosen).
        bundle: Path to JWKS bundle (if JWT auth path chosen).
        audience: Audience claim(s) for JWT verification.
        message: If provided, run in active mode sending this payload.
        iterations: Number of request/reply cycles in active mode.
        unicast: Explicitly request a UNICAST session (sticky) if True.

    Behavior:
        - Builds/Connects Slim app.
        - If message is supplied -> create session & publish + receive replies.
        - If message not supplied -> wait indefinitely for inbound sessions and echo payloads.
    """
    # Build and connect the Slim application (handles auth selection internally).
    local_app: slim_bindings.Slim = await create_local_app(
        local,
        slim,
        enable_opentelemetry=enable_opentelemetry,
        shared_secret=shared_secret,
        jwt=jwt,
        spire_trust_bundle=spire_trust_bundle,
        audience=audience,
    )

    # Numeric unique instance ID (useful for distinguishing multiple processes).
    instance = local_app.id_str

    # If user intends to send messages, remote must be provided for routing.
    if message and not remote:
        raise ValueError("Remote ID must be provided when message is specified.")

    # ACTIVE MODE (publishing + expecting replies)
    if message and remote:
        # Convert the remote ID string into a PyName.
        remote_name = split_id(remote)
        # Establish routing so outbound publishes know the remote destination.
        await local_app.set_route(remote_name)

        # Decide whether to create UNICAST or ANYCAST session.
        # MLS implies a need for stable, peer-specific communication (UNICAST).
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

        # Iterate send->receive cycles.
        for i in range(iterations):
            try:
                if unicast or enable_mls:
                    # UNICAST publish (no dest required, fixed peer, derived from session configuration).
                    await session.publish(message.encode())
                else:
                    # ANYCAST publish (dest required, load-balanced to any available peer).
                    await session.publish_with_destination(
                        message.encode(), remote_name
                    )
                format_message_print(
                    f"{instance}",
                    f"Sent message {message} - {i + 1}/{iterations}:",
                )
                # Wait for reply from remote peer.
                _msg_ctx, reply = await session.get_message()
                format_message_print(
                    f"{instance}",
                    f"received (from session {session.id}): {reply.decode()}",
                )
            except Exception as e:
                # Surface an error but continue attempts (simple resilience).
                format_message_print(f"{instance}", f"error: {e}")
            # Basic pacing so output remains readable.
            await asyncio.sleep(1)

    # PASSIVE MODE (listen for inbound sessions)
    else:
        while True:
            format_message_print(
                f"{instance}", "waiting for new session to be established"
            )
            # Block until a remote peer initiates a session to us.
            session = await local_app.listen_for_session()
            format_message_print(f"{instance}", f"new session {session.id}")

            async def session_loop(sess: slim_bindings.PySession):  # type: ignore
                """
                Inner loop for a single inbound session:
                  * Receive messages until the session is closed or an error occurs.
                  * Echo each message back using publish_to.
                """
                while True:
                    try:
                        msg_ctx, payload = await sess.get_message()
                    except Exception:
                        # Session likely closed or transport broken.
                        break
                    text = payload.decode()
                    format_message_print(f"{instance}", f"received: {text}")
                    # Echo reply with appended instance identifier.
                    await sess.publish_to(msg_ctx, f"{text} from {instance}".encode())

            # Launch a dedicated task to handle this session (allow multiple).
            asyncio.create_task(session_loop(session))


def p2p_options(function):
    """
    Decorator adding point-to-point specific CLI options (message + iterations).

    Options:
        --message <str>     : Activate active mode and send this payload.
        --iterations <int>  : Number of request/reply cycles in active mode.
    """
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
    shared_secret: str = "secret",
    jwt: str | None = None,
    spire_trust_bundle: str | None = None,
    audience: list[str] | None = None,
    invites: list[str] | None = None,
    message: str | None = None,
    iterations: int = 1,
):
    """
    CLI entry-point for ANYCAST (non-sticky) point-to-point example.

    Behavior:
        - Forces enable_mls=False (MLS not supported for ANYCAST here).
        - Delegates execution to run_client with unicast flag False.

    Parameter notes:
        invites: Accepted only for interface parity with multicast / unicast
                 entry-points. It is ignored in ANYCAST mode (no group
                 invitations are sent for point-to-point anycast sessions).
    """
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
                spire_trust_bundle=spire_trust_bundle,
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
    shared_secret: str = "secret",
    jwt: str | None = None,
    spire_trust_bundle: str | None = None,
    audience: list[str] | None = None,
    invites: list[str] | None = None,
    message: str | None = None,
    iterations: int = 1,
):
    """
    CLI entry-point for UNICAST (sticky) point-to-point example.

    Behavior:
        - Passes enable_mls through (MLS allowed in UNICAST mode).
        - Delegates execution to run_client with unicast flag True.

    Parameter notes:
        invites: Present for signature symmetry with multicast examples; it is
                 ignored here because UNICAST sessions do not invite additional
                 participants (they are strictly 1:1).
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
                message=message,
                iterations=iterations,
                unicast=True,
            )
        )
    except KeyboardInterrupt:
        print("Client interrupted by user.")
