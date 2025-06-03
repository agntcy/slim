# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import datetime
import os

from common import format_message, split_id

import slim_bindings


async def run_client(
    local_id, remote_id, address, producer, enable_opentelemetry: bool
):
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

    # Split the IDs into their respective components
    remote_organization, remote_namespace, broadcast_topic = split_id(remote_id)

    # Get the local agent instance from env
    instance = os.getenv("SLIM_INSTANCE_ID", local_agent)

    async with slim:
        if producer:
            # Create a route to the remote ID
            await slim.set_route(remote_organization, remote_namespace, broadcast_topic)

            # create streaming session with default config
            session_info = await slim.create_session(
                slim_bindings.PySessionConfiguration.Streaming(
                    slim_bindings.PySessionDirection.SENDER,
                    topic=None,
                    max_retries=5,
                    timeout=datetime.timedelta(seconds=5),
                )
            )

            # initialize count
            count = 0

            while True:
                try:
                    # Send a message
                    print(
                        format_message(
                            f"{instance} streaming message to {remote_organization}/{remote_namespace}/{broadcast_topic}: ",
                            f"{count}",
                        )
                    )

                    # Send the message and wait for a reply
                    await slim.publish(
                        session_info,
                        f"{count}".encode(),
                        remote_organization,
                        remote_namespace,
                        broadcast_topic,
                    )

                    count += 1
                except Exception as e:
                    print("received error: ", e)
                finally:
                    await asyncio.sleep(0.5)
        else:
            # subscribe to streaming session
            await slim.subscribe(remote_organization, remote_namespace, broadcast_topic)

            # Wait for messages and not reply
            while True:
                session_info, _ = await slim.receive()
                print(
                    format_message(
                        f"{instance.capitalize()} received a new session:",
                        f"{session_info.id}",
                    )
                )

                async def background_task(session_info):
                    while True:
                        # Receive the message from the session
                        session, msg = await slim.receive(session=session_info)
                        print(
                            format_message(
                                f"{local_agent.capitalize()} received from {remote_organization}/{remote_namespace}/{broadcast_topic}: ",
                                f"{msg.decode()}",
                            )
                        )

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
    parser.add_argument(
        "-p",
        "--producer",
        action="store_true",
        default=False,
        help="Enable producer mode.",
    )

    args = parser.parse_args()

    # Run the client with the specified local ID, remote ID, and optional message
    await run_client(
        args.local,
        args.remote,
        args.slim,
        args.producer,
        args.enable_opentelemetry,
    )


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("Program terminated by user.")
