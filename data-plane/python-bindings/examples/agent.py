# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import time
import os

import agp_bindings
from agp_bindings import GatewayConfig, PySessionInfo


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
    return f"{color.BOLD}{color.CYAN}{message1.capitalize() :<45}{color.END}{message2}"


async def run_client(
    local_id, remote_id, message, address, iterations, enable_opentelemetry: bool
):
    # init tracing
    agp_bindings.init_tracing(
        log_level="info", enable_opentelemetry=enable_opentelemetry
    )

    # Split the IDs into their respective components
    try:
        local_organization, local_namespace, local_agent = local_id.split("/")
    except ValueError:
        print("Error: IDs must be in the format organization/namespace/agent.")
        return

    # create new gateway object
    gateway = await agp_bindings.Gateway.new(
        local_organization, local_namespace, local_agent
    )

    # Configure gateway
    config = GatewayConfig(endpoint=address, insecure=True)
    gateway.configure(config)

    # Connect to the service and subscribe for the local name
    print(format_message(f"connecting to:", address))
    _ = await gateway.connect()
    await gateway.subscribe(
        local_organization, local_namespace, local_agent, gateway.id()
    )

    if message:
        if not iterations:
            iterations = 1

        # Split the IDs into their respective components
        try:
            remote_organization, remote_namespace, remote_agent = remote_id.split("/")
        except ValueError:
            print("Error: IDs must be in the format organization/namespace/agent.")
            return

        # Create a route to the remote ID
        await gateway.set_route(remote_organization, remote_namespace, remote_agent)

        # create a session
        session = await gateway.create_ff_session(
            agp_bindings.PyFireAndForgetConfiguration()
        )

        for i in range(0, iterations):
            try:
                # Send the message
                await gateway.publish(
                    session,
                    message.encode(),
                    remote_organization,
                    remote_namespace,
                    remote_agent,
                )
                print(format_message(f"{local_agent} sent:", message))

                # Wait for a reply
                session_info, msg = await gateway.receive(session=session.id)
                print(
                    format_message(
                        f"{local_agent.capitalize()} received (from session {session_info.id}):",
                        f"{msg.decode()}",
                    )
                )
            except Exception as e:
                print("received error: ", e)

            time.sleep(1)
    else:
        # Get the local agent instance from env
        instance = os.getenv("AGP_INSTANCE_ID", local_agent)

        # Wait for a message and reply in a loop
        while True:
            session_info, _ = await gateway.receive()
            print(
                format_message(
                    f"{local_agent.capitalize()} received a new session:",
                    f"{session_info.id}",
                )
            )

            async def background_task():
                while True:
                    # Receive the message from the session
                    session, msg = await gateway.receive(session=session_info.id)
                    print(
                        format_message(
                            f"{local_agent.capitalize()} received (from session {session.id}):",
                            f"{msg.decode()}",
                        )
                    )

                    ret = f"{msg.decode()} from {instance}"

                    await gateway.publish_to(session, ret.encode())
                    print(format_message(f"{local_agent.capitalize()} replies:", ret))

            asyncio.create_task(background_task())


async def main():
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
        default="http://127.0.0.1:46357",
    )
    parser.add_argument(
        "-i",
        "--iterations",
        type=int,
        help="Number of messages to send, one per second.",
    )
    parser.add_argument(
        "-t",
        "--enable-opentelemetry",
        action="store_true",
        default=False,
        help="Enable OpenTelemetry tracing.",
    )

    args = parser.parse_args()

    # Run the client with the specified local ID, remote ID, and optional message
    await run_client(
        args.local,
        args.remote,
        args.message,
        args.gateway,
        args.iterations,
        args.enable_opentelemetry,
    )


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("Program terminated by user.")
