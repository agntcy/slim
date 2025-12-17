# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Slim server example (extensively commented).

This module demonstrates:
  * Initializing crypto provider
  * Spinning up a Slim service in server mode, on a given endpoint
  * Graceful shutdown via SIGINT (Ctrl+C)

High-level flow:
  main() -> asyncio.run(amain())
      amain():
        * Parse CLI flags (address)
        * Register SIGINT handler that signals an asyncio.Event
        * Launch server_task() to start Slim server
        * Wait until the event is set (Ctrl+C)
        * Cancel server task and perform cleanup
"""

import argparse
import asyncio
from signal import SIGINT

import slim_uniffi_bindings._slim_bindings.slim_bindings as slim

from .common import create_server_config, create_tls_config

# Global (module-level) name retained only for illustrative purposes.
# The server Slim instance is returned from run_server and captured in amain.
global slim_app


async def run_server(address: str):
    """
    Initialize crypto, construct a Slim service instance, and start
    its server endpoint.

    Args:
        address: Endpoint string (host:port form) on which to listen.

    Returns:
        BindingsAdapter: The running Slim instance (server mode).
    """
    # Initialize crypto provider
    slim.initialize_crypto_provider()

    # Build a shared-secret. Real deployments should
    # use stronger identity mechanisms (e.g. JWT + proper key management).
    app_name = slim.Name(components=["cisco", "default", "slim"], id=None)
    # Must be > 32 bytes
    shared_secret = "jasfhuejasdfhays3wtkrktasdhfsadu2rtkdhsfgeht"

    # Create Slim instance
    app = slim.create_app_with_secret(app_name, shared_secret)

    # Create server config with insecure TLS (development setting).
    tls_config = create_tls_config(insecure=True)
    server_config = create_server_config(address, tls_config)

    # Launch the embedded server (async version).
    await app.run_server_async(server_config)
    return app


async def amain():
    """
    Async entry point for CLI usage.

    Steps:
        1. Parse command-line arguments.
        2. Register a SIGINT (Ctrl+C) handler setting an asyncio.Event.
        3. Start the server in a background task.
        4. Wait until the event is triggered.
        5. Cancel the background task and finalize gracefully.
    """
    parser = argparse.ArgumentParser(description="Command line Slim server example.")
    parser.add_argument(
        "-s", "--slim", type=str, help="Slim address.", default="127.0.0.1:46357"
    )

    args = parser.parse_args()

    # Event used to signal shutdown from SIGINT.
    stop_event = asyncio.Event()
    # Keep a reference to the Slim instance (might be used for future cleanup hooks).
    slim_ref = None

    def shutdown():
        """
        Signal handler callback.
        Sets the stop_event to begin shutdown sequence.
        """
        print("\nShutting down...")
        stop_event.set()

    # Register signal handler for Ctrl+C.
    loop = asyncio.get_running_loop()
    loop.add_signal_handler(SIGINT, shutdown)

    async def server_task():
        """
        Background task that launches the Slim server and keeps it running.
        """
        nonlocal slim_ref
        slim_ref = await run_server(args.slim)

    # Start server concurrently.
    task = asyncio.create_task(server_task())

    # Block until shutdown is requested.
    await stop_event.wait()

    # Cancel server task (will propagate cancellation if still awaiting).
    task.cancel()
    try:
        await task
    except asyncio.CancelledError:
        # Expected when shutting down gracefully.
        pass


def main():
    """
    Synchronous wrapper enabling `python -m slim_bindings_examples.slim`
    or console-script entry point usage.
    """
    try:
        asyncio.run(amain())
    except KeyboardInterrupt:
        # Fallback if signal handling did not intercept first (rare edge cases).
        print("Program terminated by user.")
