# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import datetime
import os

from .common import (
    split_id,
    common_parser,
    get_provider_and_verifier_with_spire,
    get_provider_and_verifier_from_shared_secret,
)

import slim_bindings


async def run_client(
    local_id: str,
    remote_id: str,
    address: str,
    moderator: bool,
    jwt: str | None,
    key: str | None,
    shared_secret: str,
    enable_opentelemetry: bool,
    mls_enabled: bool,
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

    # Derive identity provider and verifier from JWK and JWT
    if jwt and key:
        provider, verifier = get_provider_and_verifier_with_spire(
            jwt,
            key,
            aud=["slim-demo"],
        )
    else:
        provider, verifier = get_provider_and_verifier_from_shared_secret(
            identity=local_id,
            shared_secret=shared_secret,
        )

    # Split the local IDs into their respective components
    local_organization, local_namespace, local_agent = split_id(local_id)

    # split local agent into participant name and id
    local_agent_name, local_agent_id = local_agent.split("-")

    # Split the remote IDs into their respective components
    remote_organization, remote_namespace, broadcast_topic = split_id(remote_id)

    name = f"{local_agent}"

    print(f"Creating participant {name}...")

    participant = await slim_bindings.Slim.new(
        local_organization, local_namespace, local_agent, provider, verifier
    )

    print(f"{name} -> Created participant {local_agent}")

    # Connect to slim server
    _ = await participant.connect({"endpoint": address, "tls": {"ca_file": "/svids/svid_bundle.pem", "insecure_skip_verify": True}})

    print(f"{name} -> Connected to to {address}")

    # set route for the chat, so that messages can be sent to the other participants
    await participant.set_route(remote_organization, remote_namespace, broadcast_topic)

    # Subscribe to the producer topic
    await participant.subscribe(remote_organization, remote_namespace, broadcast_topic)

    if moderator:
        print(f"{name} -> Creating new pubsub sessions...")
        # create pubsubb session. A pubsub session is a just a bidirectional
        # streaming session, where participants are both sender and receivers
        session_info = await participant.create_session(
            slim_bindings.PySessionConfiguration.Streaming(
                slim_bindings.PySessionDirection.BIDIRECTIONAL,
                topic=slim_bindings.PyAgentType(
                    remote_organization, remote_namespace, broadcast_topic
                ),
                moderator=True,
                max_retries=5,
                timeout=datetime.timedelta(seconds=5),
                mls_enabled=mls_enabled,
            )
        )

        # invite all participants
        for i in range(1, int(local_agent_id)):
            type_to_add = f"participant-{i}"
            to_add = slim_bindings.PyAgentType(remote_organization, remote_namespace, type_to_add)
            await participant.set_route(remote_organization, remote_namespace, type_to_add)
            await participant.invite(session_info, to_add)
            print(f"{name} -> add {type_to_add} to the group")

    tasks = []

    # define the background task
    async def background_task():
        msg = f"Hello from {local_agent}"

        async with participant:
            # init session from session
            if moderator:
                recv_session = session_info
            else:
                print(f"{name} -> Waiting for session...")
                recv_session, _ = await participant.receive()

            while True:
                try:
                    # receive message from session
                    recv_session, msg_rcv = await participant.receive(
                        session=recv_session.id
                    )

                    # print received message
                    print(f"{name} -> Received message: {msg_rcv.decode()}")
                except asyncio.CancelledError:
                    break
                except Exception as e:
                    print(f"{name} -> Error receiving message: {e}")
                    break

    tasks.append(asyncio.create_task(background_task()))

    if moderator:
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

        tasks.append(asyncio.create_task(background_task_keyboard()))

    # Wait for both tasks to finish
    await asyncio.gather(*tasks)


async def amain():
    parser = common_parser()

    parser.add_argument(
        "--moderator",
        "-m",
        action="store_true",
        default=os.getenv("SLIM_MODERATOR", "false").lower() == "true",
        help="Enable moderator mode.",
    )

    parser.add_argument(
        "-x",
        "--mls-enabled",
        action="store_true",
        default=os.getenv("SLIM_MLS_ENABLED", "false").lower() == "true",
        help="Disable MLS (Message Layer Security) for the pubsub session.",
    )

    args = parser.parse_args()

    # Run the client with the specified local ID, remote ID, and optional message
    await run_client(
        args.local,
        args.remote,
        args.slim,
        args.moderator,
        args.jwt,
        args.key,
        args.shared_secret,
        args.enable_opentelemetry,
        args.mls_enabled,
    )


def main():
    try:
        asyncio.run(amain())
    except KeyboardInterrupt:
        print("Program terminated by user.")
