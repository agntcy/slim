# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import datetime

from common import split_id

import slim_bindings


async def run_client(local_id, remote_id, address, enable_opentelemetry: bool):
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

    # Split the local IDs into their respective components
    local_organization, local_namespace, local_agent = split_id(local_id)

    # Split the remote IDs into their respective components
    remote_organization, remote_namespace, broadcast_topic = split_id(remote_id)

    name = f"{local_agent}"

    print(f"Creating participant {name}...")

    participant = await slim_bindings.Slim.new(
        local_organization, local_namespace, local_agent
    )

    # Connect to slim server
    _ = await participant.connect({"endpoint": address, "tls": {"insecure": True}})

    # set route for the chat, so that messages can be sent to the other participants
    await participant.set_route(remote_organization, remote_namespace, broadcast_topic)

    # Subscribe to the producer topic
    await participant.subscribe(remote_organization, remote_namespace, broadcast_topic)

    print(f"{name} -> Creating new pubsub sessions...")
    # create pubsubb session. A pubsub session is a just a bidirectional
    # streaming session, where participants are both sender and receivers
    session_info = await participant.create_session(
        slim_bindings.PySessionConfiguration.Streaming(
            slim_bindings.PySessionDirection.BIDIRECTIONAL,
            topic=slim_bindings.PyAgentType(
                remote_organization, remote_namespace, broadcast_topic
            ),
            max_retries=5,
            timeout=datetime.timedelta(seconds=5),
        )
    )

    # define the background task
    async def background_task():
        msg = f"Hello from {local_agent}"

        async with participant:
            while True:
                try:
                    # receive message from session
                    recv_session, msg_rcv = await participant.receive(
                        session=session_info.id
                    )

                    # Check if the message is calling this specific participant
                    # if not, ignore it
                    if local_agent in msg_rcv.decode():
                        # print the message
                        print(f"{name} -> Received message for me: {msg_rcv.decode()}")

                        # send the message to the next participant
                        await participant.publish(
                            recv_session,
                            msg.encode(),
                            remote_organization,
                            remote_namespace,
                            broadcast_topic,
                        )

                        print(f"{name} -> Sending message to all participants: {msg}")
                    else:
                        print(
                            f"{name} -> Receiving message: {msg_rcv.decode()} - not for me."
                        )
                except asyncio.CancelledError:
                    break
                except Exception as e:
                    print(f"{name} -> Error receiving message: {e}")
                    break

    receive_task = asyncio.create_task(background_task())

    async def background_task_keyboard():
        while True:
            user_input = await asyncio.to_thread(input, "message> ")
            if user_input == "exit":
                break

            # Send the message to the all participants
            await participant.publish(
                session_info,
                f"{user_input}".encode(),
                remote_organization,
                remote_namespace,
                broadcast_topic,
            )

        receive_task.cancel()

    send_task = asyncio.create_task(background_task_keyboard())

    # Wait for both tasks to finish
    await asyncio.gather(receive_task, send_task)


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
        help="Slim address.",
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
        args.slim,
        args.enable_opentelemetry,
    )


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("Program terminated by user.")
