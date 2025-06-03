# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import os

from common import format_message, split_id

import slim_bindings


async def run_client(local_id, remote_id, message, address, enable_opentelemetry: bool):
    # init tracing
    slim_bindings.init_tracing(
        {
            "log_level": "info",
            "opentelemetry": {
                "enabled": enable_opentelemetry,
                "grpc": {
                    "endpoint": "http://localhost:4317",
                },
            },
        }
    )

    # Split the IDs into their respective components
    local_organization, local_namespace, local_agent = split_id(local_id)

    # create new slim object
    slim = await slim_bindings.Slim.new(
        local_organization, local_namespace, local_agent
    )

    # Connect to the service and subscribe for the local name
    print(format_message("connecting to:", address))
    _ = await slim.connect({"endpoint": address, "tls": {"insecure": True}})

    # Get the local agent instance from env
    instance = os.getenv("SLIM_INSTANCE_ID", local_agent)

    async with slim:
        if message:
            # Split the IDs into their respective components
            remote_organization, remote_namespace, remote_agent = split_id(remote_id)

            # Create a route to the remote ID
            await slim.set_route(remote_organization, remote_namespace, remote_agent)

            # create a session
            session = await slim.create_session(
                slim_bindings.PySessionConfiguration.RequestResponse()
            )

            try:
                # Send the message and wait for a reply
                session_info, reply = await slim.request_reply(
                    session,
                    message.encode(),
                    remote_organization,
                    remote_namespace,
                    remote_agent,
                )
                print(format_message(f"{instance} sent:", message))

                # Print reply and exit
                print(
                    format_message(
                        f"{instance} received (from session {session_info.id}):",
                        f"{reply.decode()}",
                    )
                )

            except Exception as e:
                print("received error: ", e)
        else:
            # Wait for a message and reply in a loop
            while True:
                session_info, _ = await slim.receive()
                print(
                    format_message(
                        f"{instance.capitalize()} received a new session:",
                        f"{session_info.id}",
                    )
                )

                async def background_task(session_id):
                    while True:
                        # Receive the message from the session
                        session, msg = await slim.receive(session=session_id)
                        print(
                            format_message(
                                f"{instance.capitalize()} received (from session {session_id}):",
                                f"{msg.decode()}",
                            )
                        )

                        ret = f"{msg.decode()} from {instance}"

                        await slim.publish_to(session, ret.encode())
                        print(format_message(f"{instance.capitalize()} replies:", ret))

                asyncio.create_task(background_task(session_info.id))


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
        "-s",
        "--slim",
        type=str,
        help="SLIM address.",
        default="http://127.0.0.1:46357",
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
        args.slim,
        args.enable_opentelemetry,
    )


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("Program terminated by user.")
