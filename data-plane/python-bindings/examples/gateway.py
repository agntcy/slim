# SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
from signal import SIGINT

import agp_bindings

# Create a service
gateway = agp_bindings.Gateway()


async def run_server(address: str):
    # init tracing
    agp_bindings.init_tracing(log_level="debug")

    # Run as server
    await gateway.serve(address, insecure=True)


async def main():
    parser = argparse.ArgumentParser(
        description="Command line client for gateway server."
    )
    parser.add_argument(
        "-g", "--gateway", type=str, help="Gateway address.", default="127.0.0.1:12345"
    )

    args = parser.parse_args()

    # Create an asyncio event to keep the loop running until interrupted
    stop_event = asyncio.Event()

    # Define a shutdown handler to set the event when interrupted
    def shutdown():
        print("\nShutting down...")
        stop_event.set()

    # Register the shutdown handler for SIGINT
    loop = asyncio.get_running_loop()
    loop.add_signal_handler(SIGINT, shutdown)

    # Run the client task
    client_task = asyncio.create_task(run_server(args.gateway))

    # Wait until the stop event is set
    await stop_event.wait()

    # Cancel the client task
    client_task.cancel()
    try:
        await client_task
    except asyncio.CancelledError:
        pass


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("Program terminated by user.")
