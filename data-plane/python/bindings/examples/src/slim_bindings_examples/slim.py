# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Slim server example (extensively commented).

This module demonstrates:
  * Initializing tracing (optionally enabling OpenTelemetry export)
  * Spinning up a Slim service in server mode, on a given endpoint
  * Graceful shutdown via SIGINT (Ctrl+C)

High-level flow:
  main() -> asyncio.run(amain())
      amain():
        * Parse CLI flags (address, OTEL toggle)
        * Register SIGINT handler that signals an asyncio.Event
        * Launch server_task() to start Slim server
        * Wait until the event is set (Ctrl+C)
        * Cancel server task and perform cleanup

Tracing:
  When --enable-opentelemetry is passed, OTEL export is enabled towards
  localhost:4317 (default OTLP gRPC collector). If no collector is running,
  tracing initialization will still succeed but spans may be dropped.
"""

import argparse
import asyncio
from signal import SIGINT

import slim_bindings

from .common import shared_secret_identity

# Global (module-level) name retained only for illustrative purposes.
# The server Slim instance is returned from run_server and captured in amain.
global slim


async def run_server(address: str, enable_opentelemetry: bool):
    """
    Initialize tracing, construct a Slim service instance, and start
    its server endpoint.

    Args:
        address: Endpoint string (host:port form) on which to listen.
        enable_opentelemetry: Whether to enable OTEL export in tracing init.

    Returns:
        Slim: The running Slim instance (server mode).
    """
    # Initialize tracing (async function; returns immediately after setup).
    await slim_bindings.init_tracing(
        {
            "log_level": "debug",
            "opentelemetry": {
                "enabled": enable_opentelemetry,
                "grpc": {
                    "endpoint": "http://localhost:4317",
                },
            },
        }
    )

    # Build a shared-secret provider/verifier pair. Real deployments should
    # use stronger identity mechanisms (e.g. JWT + proper key management).
    provider, verifier = shared_secret_identity(
        identity="slim",
        secret="jasfhuejasdfhays3wtkrktasdhfsadu2rtkdhsfgeht",  # Must be > 32 bytes
    )

    # Create Slim instance with a fixed Name. Organization/namespace/app are illustrative.
    slim = slim_bindings.Slim(
        slim_bindings.Name("cisco", "default", "slim"), provider, verifier
    )

    # Launch the embedded server with insecure TLS (development setting).
    await slim.run_server({"endpoint": address, "tls": {"insecure": True}})
    return slim


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
        "-s", "--slim", type=str, help="Slim address.", default="127.0.0.1:12345"
    )
    parser.add_argument(
        "--enable-opentelemetry",
        "-t",
        action="store_true",
        default=False,
        help="Enable OpenTelemetry tracing.",
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
        slim_ref = await run_server(args.slim, args.enable_opentelemetry)

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
