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
import sys
from signal import SIGINT

import slim_bindings

from .common import setup_service
from .config import ServerConfig, load_config_with_cli_override


async def amain(config: ServerConfig):
    """
    Async entry point for server.

    Steps:
        1. Initialize tracing and global service.
        2. Start the server (Rust manages the server lifecycle).
        3. Register SIGINT handler and wait for shutdown signal.
        4. Stop the server gracefully.

    Args:
        config: ServerConfig instance containing all configuration.
    """
    # Get the global service instance
    service = setup_service()

    # Launch the embedded server with insecure TLS (development setting).
    # The server runs in the Rust runtime and is managed there.
    server_config = slim_bindings.new_insecure_server_config(config.slim)
    await service.run_server_async(server_config)

    print(f"Slim server started on {config.slim}")
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
        print(f"Server stopped at {config.slim}")
    except Exception as e:
        print(f"Error stopping server: {e}")


def main():
    """
    CLI entry-point for the server example.

    Parses command-line arguments and config file, then runs the server.
    """
    parser = argparse.ArgumentParser(
        description="SLIM Server Example\n\n"
        "Start a SLIM server for handling client connections.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )

    parser.add_argument(
        "-s",
        "--slim",
        type=str,
        default="127.0.0.1:12345",
        help="SLIM server address (host:port) (default: 127.0.0.1:12345)",
    )

    parser.add_argument(
        "--enable-opentelemetry",
        "-t",
        action="store_true",
        help="Enable OpenTelemetry tracing",
    )

    parser.add_argument(
        "--config",
        type=str,
        help="Path to configuration file (JSON, YAML, or TOML)",
    )

    # Parse arguments
    args = parser.parse_args()

    # Convert to dictionary
    args_dict = vars(args)

    # Load configuration (CLI args override env vars and config file)
    try:
        config = load_config_with_cli_override(ServerConfig, args_dict)
    except Exception as e:
        print(f"Configuration error: {e}", file=sys.stderr)
        sys.exit(1)

    # Run the server
    try:
        asyncio.run(amain(config))
    except KeyboardInterrupt:
        print("\nServer terminated by user.")
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
