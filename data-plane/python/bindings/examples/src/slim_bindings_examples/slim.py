# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
from signal import SIGINT

import slim_bindings

from .common import shared_secret_identity


async def run_server(address: str, enable_opentelemetry: bool):
    # init tracing
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

    # not used in the slim server
    provider, verifier = shared_secret_identity(
        identity="slim",
        secret="secret",
    )
    # create new slim object (local variable)
    slim = await slim_bindings.Slim.new(
        slim_bindings.PyName("cisco", "default", "slim"), provider, verifier
    )

    # Run as server
    await slim.run_server({"endpoint": address, "tls": {"insecure": True}})
    return slim


async def amain():
    parser = argparse.ArgumentParser(description="Command line client for slim server.")
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

    stop_event = asyncio.Event()
    slim_ref = None  # Keep reference to slim server

    def shutdown():
        print("\nShutting down...")
        stop_event.set()

    loop = asyncio.get_running_loop()
    loop.add_signal_handler(SIGINT, shutdown)

    async def server_task():
        nonlocal slim_ref
        slim_ref = await run_server(args.slim, args.enable_opentelemetry)

    task = asyncio.create_task(server_task())

    await stop_event.wait()

    task.cancel()
    try:
        await task
    except asyncio.CancelledError:
        pass


def main():
    try:
        asyncio.run(amain())
    except KeyboardInterrupt:
        print("Program terminated by user.")
