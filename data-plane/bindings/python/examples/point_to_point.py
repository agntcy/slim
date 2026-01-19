# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Point-to-point messaging example for Slim bindings.

This example can operate in two primary modes:

1. Active sender (message mode):
   - Creates a session.
   - Publishes a fixed or user-supplied message multiple times to a remote identity.
   - Receives replies for each sent message (request/reply pattern).

2. Passive listener (no --message provided):
   - Waits for inbound sessions initiated by a remote party.
   - Echoes replies for each received payload, tagging them with the local instance ID.

Key concepts demonstrated:
  - Global service usage with create_app_with_secret()
  - PointToPoint session creation logic.
  - Publish / receive loop with per-message reply.
  - Simple flow control via iteration count and sleeps (demo-friendly).

Notes:
  * PointToPoint sessions stick to one specific peer (sticky / affinity semantics).

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
    message: str | None = None,
    iterations: int = 1,
):
    """
    Core coroutine that performs either active send or passive listen logic.

    Args:
        local: Local identity string (org/namespace/app).
        remote: Remote identity target for routing / session initiation.
        enable_opentelemetry: Enable OpenTelemetry tracing if configured.
        enable_mls: Enable MLS.
        shared_secret: Shared secret for symmetric auth (dev/demo).
        jwt: Path to static JWT token (if JWT auth path chosen).
        spire_trust_bundle: SPIRE trust bundle file path.
        audience: Audience claim(s) for JWT verification.
        spire_socket_path: SPIRE Workload API socket path.
        spire_target_spiffe_id: Target SPIFFE ID for SPIRE.
        spire_jwt_audience: Audience(s) for SPIRE JWT SVID.
        message: If provided, run in active mode sending this payload.
        iterations: Number of request/reply cycles in active mode.

    Behavior:
        - Builds Slim app using global service.
        - If message is supplied -> create session & publish + receive replies.
        - If message not supplied -> wait indefinitely for inbound sessions and echo payloads.
    """
    # Build the Slim application using global service
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

    # Numeric unique instance ID (useful for distinguishing multiple processes).
    instance = str(local_app.id())

    # If user intends to send messages, remote must be provided for routing.
    if message and not remote:
        raise ValueError("Remote ID must be provided when message is specified.")

    # ACTIVE MODE (publishing + expecting replies)
    if message and remote:
        # Convert the remote ID string into a Name.
        remote_name = split_id(remote)

        # Create point-to-point session configuration
        config = slim_bindings.SessionConfig(
            session_type=slim_bindings.SessionType.POINT_TO_POINT,
            enable_mls=enable_mls,
            max_retries=5,
            interval=datetime.timedelta(seconds=5),
            metadata={},
        )

        # Create session - returns a context with completion and session
        session_context = await local_app.create_session_async(config, remote_name)
        # Wait for session to be established
        await session_context.completion.wait_async()
        session = session_context.session

        session_closed = False
        # Iterate send->receive cycles.
        for i in range(iterations):
            try:
                # Publish message to the session
                await session.publish_async(message.encode(), None, None)
                format_message_print(
                    f"{instance}",
                    f"Sent message {message} - {i + 1}/{iterations}",
                )
                # Wait for reply from remote peer.
                received_msg = await session.get_message_async(
                    timeout=datetime.timedelta(seconds=30)
                )
                reply = received_msg.payload
                format_message_print(
                    f"{instance}",
                    f"received (from session {session.id()}): {reply.decode()}",
                )
            except Exception as e:
                # Surface an error but continue attempts (simple resilience).
                format_message_print(f"{instance}", f"error: {e}")
                # if the session is closed exit
                if "session closed" in str(e).lower():
                    session_closed = True
                    break
            # Basic pacing so output remains readable.
            await asyncio.sleep(1)

        if not session_closed:
            # Delete session
            handle = await local_app.delete_session_async(session)
            await handle.wait_async()

    # PASSIVE MODE (listen for inbound sessions)
    else:
        while True:
            format_message_print(
                f"{instance}", "waiting for new session to be established"
            )
            # Block until a remote peer initiates a session to us.
            session_context = await local_app.listen_for_session_async(None)
            session = session_context
            format_message_print(f"{instance}", f"new session {session.id()}")

            async def session_loop(sess: slim_bindings.Session):
                """
                Inner loop for a single inbound session:
                  * Receive messages until the session is closed or an error occurs.
                  * Echo each message back using publish.
                """
                while True:
                    try:
                        received_msg = await sess.get_message_async(
                            timeout=datetime.timedelta(seconds=30)
                        )
                        payload = received_msg.payload
                    except Exception:
                        # Session likely closed or transport broken.
                        break
                    text = payload.decode()
                    format_message_print(f"{instance}", f"received: {text}")
                    # Echo reply with appended instance identifier.
                    await sess.publish_async(
                        f"{text} from {instance}".encode(), None, None
                    )

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
def main(
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
    message: str | None = None,
    iterations: int = 1,
):
    """
    CLI entry-point for point-to-point example.

    Parameter notes:
        invites: Present for signature symmetry with group examples; it is
                 ignored here because p2p sessions do not invite additional
                 participants (they are strictly 1:1).
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
                message=message,
                iterations=iterations,
            )
        )
    except KeyboardInterrupt:
        print("Client interrupted by user.")
