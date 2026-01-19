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
import sys

import slim_bindings

from .common import (
    create_base_parser,
    create_local_app,
    format_message_print,
    parse_args_to_dict,
    split_id,
)
from .config import PointToPointConfig, load_config_with_cli_override


async def run_client(config: PointToPointConfig):
    """
    Core coroutine that performs either active send or passive listen logic.

    Args:
        config: PointToPointConfig instance containing all configuration.

    Behavior:
        - Builds Slim app using global service.
        - If message is supplied -> create session & publish + receive replies.
        - If message not supplied -> wait indefinitely for inbound sessions and echo payloads.
    """
    # Build the Slim application using global service
    local_app, conn_id = await create_local_app(config)

    # Numeric unique instance ID (useful for distinguishing multiple processes).
    instance = str(local_app.id())

    # If user intends to send messages, remote must be provided for routing.
    if config.message and not config.remote:
        raise ValueError("Remote ID must be provided when message is specified.")

    # ACTIVE MODE (publishing + expecting replies)
    if config.message and config.remote:
        # Convert the remote ID string into a Name.
        remote_name = split_id(config.remote)

        # Create local route to enable forwarding towards remote name
        await local_app.set_route_async(remote_name, conn_id)

        # Create point-to-point session configuration
        session_config = slim_bindings.SessionConfig(
            session_type=slim_bindings.SessionType.POINT_TO_POINT,
            enable_mls=config.enable_mls,
            max_retries=5,
            interval=datetime.timedelta(seconds=5),
            metadata={},
        )

        # Create session - returns a context with completion and session
        session_context = await local_app.create_session_async(
            session_config, remote_name
        )
        # Wait for session to be established
        await session_context.completion.wait_async()
        session = session_context.session

        session_id = session_context.session.session_id()

        session_closed = False
        # Iterate send->receive cycles.
        for i in range(config.iterations):
            try:
                # Publish message to the session
                await session.publish_async(config.message.encode(), None, None)
                format_message_print(
                    f"{instance}",
                    f"Sent message {config.message} - {i + 1}/{config.iterations}",
                )
                # Wait for reply from remote peer.
                received_msg = await session.get_message_async(
                    timeout=datetime.timedelta(seconds=30)
                )
                reply = received_msg.payload
                format_message_print(
                    f"{instance}",
                    f"received (from session {session_id}): {reply.decode()}",
                )
            except Exception as e:
                # Surface an error but continue attempts
                format_message_print(f"{instance}", f"error: {e}")
                # if the session is closed exit
                if "session closed" in str(e).lower():
                    session_closed = True
                    break
            # 1s pacing so output remains readable.
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
            session = await local_app.listen_for_session_async(None)
            session_id = session.session_id()
            format_message_print(f"{instance}", f"new session {session_id}")

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


def main():
    """
    CLI entry-point for point-to-point example.

    Parses command-line arguments and config file, then runs the client.
    """
    # Create parser with common options
    parser = create_base_parser(
        description="SLIM Point-to-Point Messaging Example\n\n"
        "Send messages to a specific peer or listen for incoming connections."
    )

    # Add point-to-point specific options
    parser.add_argument(
        "--message",
        type=str,
        help="Message to send (activates sender mode)",
    )

    parser.add_argument(
        "--iterations",
        type=int,
        default=10,
        help="Number of request/reply cycles in sender mode (default: 10)",
    )

    # Parse arguments
    args = parser.parse_args()

    # Convert to dictionary
    args_dict = parse_args_to_dict(args)

    # Load configuration (CLI args override env vars and config file)
    try:
        config = load_config_with_cli_override(PointToPointConfig, args_dict)
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
