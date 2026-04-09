# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import logging
import os
from uuid import uuid4

from dotenv import load_dotenv

load_dotenv()

import httpx
import slim_bindings
from a2a.client import Client, ClientFactory, minimal_agent_card
from a2a.types import Message, Part, Role, TextPart
from slima2a.client_transport import ClientConfig, SRPCTransport, slimrpc_channel_factory

# SLIM connection
SLIM_URL = os.getenv("SLIM_URL", "http://localhost:46357")
SLIM_NAMESPACE = os.getenv("SLIM_NAMESPACE", "agntcy")
SLIM_GROUP = os.getenv("SLIM_GROUP", "demo")
SLIM_NAME = os.getenv("SLIM_NAME", "k8s_troubleshooting_agent")
SLIM_CLIENT_NAME = os.getenv("SLIM_CLIENT_NAME", "k8s_troubleshooting_client")

# Shared-secret auth (used when SPIRE is not configured)
SLIM_SECRET = os.getenv("SLIM_SECRET", "")

# SPIRE auth (takes precedence over shared secret when SPIRE_SOCKET_PATH is set)
SPIRE_SOCKET_PATH = os.getenv("SPIRE_SOCKET_PATH", "")
SPIRE_TARGET_SPIFFE_ID = os.getenv("SPIRE_TARGET_SPIFFE_ID", "") or None
SPIRE_JWT_AUDIENCE = [
    a.strip()
    for a in os.getenv("SPIRE_JWT_AUDIENCE", "").split(",")
    if a.strip()
]

logger = logging.getLogger(__name__)


async def create_slim_app() -> tuple[slim_bindings.App, slim_bindings.Name, int]:
    """Initialise SLIM and create an App using SPIRE or shared-secret auth.

    Auth resolution order:
      1. SPIRE  — when SPIRE_SOCKET_PATH is set.
      2. Shared secret — when SLIM_SECRET is set.
    """
    slim_bindings.uniffi_set_event_loop(asyncio.get_running_loop())  # type: ignore[arg-type]

    tracing_config = slim_bindings.new_tracing_config()
    runtime_config = slim_bindings.new_runtime_config()
    service_config = slim_bindings.new_service_config()
    tracing_config.log_level = "info"

    slim_bindings.initialize_with_configs(
        tracing_config=tracing_config,
        runtime_config=runtime_config,
        service_config=[service_config],
    )

    service = slim_bindings.get_global_service()
    local_name = slim_bindings.Name(SLIM_NAMESPACE, SLIM_GROUP, SLIM_CLIENT_NAME)

    client_config = slim_bindings.new_insecure_client_config(SLIM_URL)
    conn_id = await service.connect_async(client_config)

    if SPIRE_SOCKET_PATH:
        logger.debug("Using SPIRE dynamic identity authentication.")
        spire_config = slim_bindings.SpireConfig(
            trust_domains=[],
            socket_path=SPIRE_SOCKET_PATH,
            target_spiffe_id=SPIRE_TARGET_SPIFFE_ID,
            jwt_audiences=SPIRE_JWT_AUDIENCE,
        )
        provider_config = slim_bindings.IdentityProviderConfig.SPIRE(config=spire_config)
        verifier_config = slim_bindings.IdentityVerifierConfig.SPIRE(config=spire_config)
        local_app = service.create_app(local_name, provider_config, verifier_config)
    else:
        logger.debug("Using shared-secret authentication.")
        local_app = service.create_app_with_secret(local_name, SLIM_SECRET)

    await local_app.subscribe_async(local_name, conn_id)

    return local_app, local_name, conn_id


async def send_message(client: Client, text: str) -> str:
    request = Message(
        role=Role.user,
        message_id=str(uuid4()),
        parts=[Part(root=TextPart(text=text))],
    )

    output = ""
    async for event in client.send_message(request=request):
        if isinstance(event, Message):
            for part in event.parts:
                if isinstance(part.root, TextPart):
                    output += part.root.text
        else:
            task, update = event
            logger.debug("task (%s) status: %s", task.id, task.status.state)
            if task.status.state == "completed" and task.artifacts:
                for artifact in task.artifacts:
                    for part in artifact.parts:
                        if isinstance(part.root, TextPart):
                            output += part.root.text

    return output


async def main() -> None:
    parser = argparse.ArgumentParser(description="Send a message to the k8s troubleshooting agent")
    parser.add_argument("message", help="Message to send to the agent")
    parser.add_argument("--log-level", default="ERROR")
    args = parser.parse_args()

    logging.basicConfig(level=args.log_level)

    local_app, local_name, conn_id = await create_slim_app()

    client_config = ClientConfig(
        supported_transports=["slimrpc"],
        httpx_client=httpx.AsyncClient(),
        slimrpc_channel_factory=slimrpc_channel_factory(local_app, conn_id),
    )
    client_factory = ClientFactory(client_config)
    client_factory.register("slimrpc", SRPCTransport.create)  # type: ignore[arg-type]

    agent_name = f"{SLIM_NAMESPACE}/{SLIM_GROUP}/{SLIM_NAME}"
    agent_card = minimal_agent_card(agent_name, ["slimrpc"])
    client = client_factory.create(card=agent_card)

    print(f"> {args.message}")
    response = await send_message(client, args.message)
    print(response)


if __name__ == "__main__":
    asyncio.run(main())
