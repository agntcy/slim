# SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import time

import agp_bindings


class color:
    PURPLE = "\033[95m"
    CYAN = "\033[96m"
    DARKCYAN = "\033[36m"
    BLUE = "\033[94m"
    GREEN = "\033[92m"
    YELLOW = "\033[93m"
    RED = "\033[91m"
    BOLD = "\033[1m"
    UNDERLINE = "\033[4m"
    END = "\033[0m"


def format_message(message1, message2):
    return f"{color.BOLD}{color.CYAN}{message1.capitalize()}{color.END}\t {message2}"


async def run_client(local_id, remote_id, message, address):
    # init tracing
    agp_bindings.init_tracing()

    # Split the IDs into their respective components
    try:
        local_organization, local_namespace, local_agent = local_id.split("/")
    except ValueError:
        print("Error: IDs must be in the format organization/namespace/agent.")
        return

    # Define the service based on the local agent
    gateway = agp_bindings.Gateway()

    # Configure gateway
    gateway.configure(endpoint=address, insecure=True)

    # Connect to the gateway server
    local_agent_id = await gateway.create_agent(
        local_organization, local_namespace, local_agent
    )

    # Connect to the service and subscribe for the local name
    _ = await gateway.connect()
    await gateway.subscribe(
        local_organization, local_namespace, local_agent, local_agent_id
    )

    if message:
        # Split the IDs into their respective components
        try:
            remote_organization, remote_namespace, remote_agent = remote_id.split("/")
        except ValueError:
            print("Error: IDs must be in the format organization/namespace/agent.")
            return

        # Create a route to the remote ID
        await gateway.set_route(remote_organization, remote_namespace, remote_agent)

        # Send the message
        await gateway.publish(
            message.encode(), remote_organization, remote_namespace, remote_agent
        )
        print(format_message(f"{local_agent.capitalize()} sent:", message))

        # Wait for a reply
        src, msg = await gateway.receive()
        print(format_message(f"{local_agent.capitalize()} received:", msg.decode()))
    else:
        # Wait for a message and reply in a loop
        while True:
            src, msg = await gateway.receive()
            print(format_message(f"{local_agent.capitalize()} received:", msg.decode()))

            ret = f"Echo from {local_agent}: {msg.decode()}"

            await gateway.publish_to(ret.encode(), src)
            print(format_message(f"{local_agent.capitalize()} replies:", ret))


def main():
    parser = argparse.ArgumentParser(
        description="Command line client for message passing."
    )
    parser.add_argument(
        "-l",
        "--local",
        type=str,
        help="Local ID in the format organization/namespace/agent.",
    )
    parser.add_argument(
        "-r",
        "--remote",
        type=str,
        help="Remote ID in the format organization/namespace/agent.",
    )
    parser.add_argument("-m", "--message", type=str, help="Message to send.")
    parser.add_argument(
        "-g",
        "--gateway",
        type=str,
        help="Gateway address.",
        default="http://127.0.0.1:12345",
    )

    args = parser.parse_args()

    # Run the client with the specified local ID, remote ID, and optional message
    asyncio.run(run_client(args.local, args.remote, args.message, args.gateway))


if __name__ == "__main__":
    main()
