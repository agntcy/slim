import argparse
import asyncio
import logging
import os
from uuid import uuid4

from dotenv import load_dotenv

load_dotenv()

import httpx
from a2a.client import Client, ClientFactory, minimal_agent_card
from a2a.types import Message, Part, Role, TextPart
from slima2a import setup_slim_client
from slima2a.client_transport import ClientConfig, SRPCTransport, slimrpc_channel_factory

SLIM_URL = os.getenv("SLIM_URL", "http://localhost:46357")
SLIM_NAMESPACE = os.getenv("SLIM_NAMESPACE", "agntcy")
SLIM_GROUP = os.getenv("SLIM_GROUP", "demo")
SLIM_NAME = os.getenv("SLIM_NAME", "k8s_troubleshooting_agent")
SLIM_SECRET = os.getenv("SLIM_SECRET", "secretsecretsecretsecretsecretsecret")
SLIM_CLIENT_NAME = os.getenv("SLIM_CLIENT_NAME", "k8s_troubleshooting_client")

logger = logging.getLogger(__name__)


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

    service, local_app, local_name, conn_id = await setup_slim_client(
        namespace=SLIM_NAMESPACE,
        group=SLIM_GROUP,
        name=SLIM_CLIENT_NAME,
        slim_url=SLIM_URL,
        secret=SLIM_SECRET,
    )

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
