# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Slim server example (extensively commented).

This module demonstrates:
  * Initializing tracing (optionally enabling OpenTelemetry export)
  * Spinning up a Slim service in server mode using the global service
  * Graceful shutdown via SIGINT (Ctrl+C)

High-level flow:
  main() -> asyncio.run(amain())
      amain():
        * Parse CLI flags (address, OTEL toggle)
        * Initialize global state and tracing
        * Start the Slim server (managed by Rust runtime)
        * Register SIGINT handler for graceful shutdown
        * Wait until Ctrl+C is pressed
        * Stop the server

Tracing:
  When --enable-opentelemetry is passed, OTEL export is enabled towards
  localhost:4317 (default OTLP gRPC collector). If no collector is running,
  tracing initialization will still succeed but spans may be dropped.
"""

import argparse
import asyncio
from signal import SIGINT

import slim_bindings


async def amain():
    """
    Async entry point for CLI usage.

    Steps:
        1. Parse command-line arguments.
        2. Initialize tracing and global service.
        3. Start the server (Rust manages the server lifecycle).
        4. Register SIGINT handler and wait for shutdown signal.
        5. Stop the server gracefully.
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

    # Initialize tracing and global state
    print("Initializing Slim server...")
    tracing_config = slim_bindings.new_tracing_config()
    runtime_config = slim_bindings.new_runtime_config()
    service_config = slim_bindings.new_service_config()

    tracing_config.log_level = "info"

    if args.enable_opentelemetry:
        # Note: OpenTelemetry configuration through config objects
        # For full OTEL support, set environment variables:
        # OTEL_EXPORTER_OTLP_ENDPOINT, OTEL_SERVICE_NAME, etc.
        print("OpenTelemetry tracing enabled (use OTEL_* env vars for configuration)")

    slim_bindings.initialize_with_configs(
        tracing_config=tracing_config,
        runtime_config=runtime_config,
        service_config=[service_config],
    )

    # Get the global service instance
    service = slim_bindings.get_global_service()

    # Launch the embedded server with insecure TLS (development setting).
    # The server runs in the Rust runtime and is managed there.
    server_config = slim_bindings.new_insecure_server_config(args.slim)
    await service.run_server_async(server_config)

    print(f"Slim server started on {args.slim}")
    print("Press Ctrl+C to stop the server")

    # Event used to signal shutdown from SIGINT.
    stop_event = asyncio.Event()

    def shutdown():
        """
        Signal handler callback.
        Sets the stop_event to begin shutdown sequence.
        """
        print("\nShutting down server...")
        stop_event.set()

    # Register signal handler for Ctrl+C.
    loop = asyncio.get_running_loop()
    loop.add_signal_handler(SIGINT, shutdown)

    # Block until shutdown is requested.
    # The server is running in the Rust runtime, so we just wait here.
    await stop_event.wait()

    # Stop the server gracefully
    try:
        await service.shutdown_async()
        print(f"Server stopped at {args.slim}")
    except Exception as e:
        print(f"Error stopping server: {e}")


def main():
    """
    Synchronous wrapper enabling `python -m slim_bindings_examples.slim`
    or console-script entry point usage.
    """
    try:
        asyncio.run(amain())
    except KeyboardInterrupt:
        print("\nProgram terminated by user.")


if __name__ == "__main__":
    main()
