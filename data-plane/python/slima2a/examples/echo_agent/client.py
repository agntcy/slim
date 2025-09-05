import argparse
import asyncio
import logging
from uuid import uuid4

import httpx
import srpc
from a2a.client import (
    A2ACardResolver,
    Client,
    ClientFactory,
    minimal_agent_card,
)
from a2a.types import (
    AgentCard,
    Message,
    Part,
    Role,
    TextPart,
)
from a2a.utils.constants import (
    AGENT_CARD_WELL_KNOWN_PATH,
)

from slima2a.client_transport import ClientConfig, SRPCTransport

BASE_URL = "http://localhost:9999"

logger = logging.getLogger(__name__)


async def fetch_agent_card(resolver: A2ACardResolver) -> AgentCard:
    agent_card: AgentCard | None = None

    try:
        logger.info(f"fetching agent card from: {BASE_URL}{AGENT_CARD_WELL_KNOWN_PATH}")
        agent_card = await resolver.get_agent_card()
        logger.info(
            f"fetched agent card: {agent_card.model_dump_json(indent=2, exclude_none=True)}",
        )

    except Exception as e:
        logger.error(f"failed fetching public agent card: {e}", exc_info=True)
        raise RuntimeError("failed fetching public agent card") from e

    return agent_card


async def main() -> None:
    args = parse_arguments()

    logging.basicConfig(level=args.log_level)

    httpx_client = httpx.AsyncClient()

    def channel_factory(topic: str) -> srpc.Channel:
        channel = srpc.Channel(
            local="agntcy/demo/client",
            remote=topic,
            slim={
                "endpoint": "http://localhost:46357",
                "tls": {
                    "insecure": True,
                },
            },
            shared_secret="secret",
        )
        return channel

    client_config = ClientConfig(
        supported_transports=["JSONRPC", "srpc"],
        streaming=args.stream,
        httpx_client=httpx_client,
        srpc_channel_factory=channel_factory,
    )
    client_factory = ClientFactory(client_config)
    client_factory.register("srpc", SRPCTransport.create)

    agent_card = None
    match args.type:
        case "srpc":
            agent_card = minimal_agent_card("agntcy/demo/echo_agent", ["srpc"])
        case "starlette":
            agent_card = await fetch_agent_card(
                resolver=A2ACardResolver(
                    httpx_client=httpx_client,
                    base_url=BASE_URL,
                )
            )

    client = client_factory.create(card=agent_card)
    logger.info("A2AClient initialized.")

    response_text = await send_message(client, args.text)
    print(f"> {args.text}")
    print(response_text)


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()

    parser.add_argument(
        "--log-level",
        type=str,
        required=False,
        default="ERROR",
    )
    parser.add_argument(
        "--stream",
        action="store_true",
        required=False,
        default=False,
    )
    parser.add_argument(
        "--text",
        type=str,
        required=True,
    )
    parser.add_argument(
        "--type",
        type=str,
        required=False,
        default="srpc",
    )

    args = parser.parse_args()

    if args.type not in ["srpc", "starlette"]:
        raise ValueError(f"Invalid client type: {args.type}")

    return args


async def send_message(
    client: Client,
    text: str,
) -> str:
    request_id = str(uuid4())
    request = Message(
        role=Role.user,
        message_id=request_id,
        parts=[Part(root=TextPart(text=text))],
    )
    logger.info(f"associated request ({request_id}) with text: {text}")

    output = ""
    try:
        async for event in client.send_message(request=request):
            if isinstance(event, Message):
                for part in event.parts:
                    if isinstance(part.root, TextPart):
                        output += part.root.text
            else:
                task, update = event
                logger.info(f"task ({task.id}) status: {task.status.state}")

                if task.status.state == "completed" and task.artifacts:
                    for artifact in task.artifacts:
                        for part in artifact.parts:
                            if isinstance(part.root, TextPart):
                                output += part.root.text

                if update:
                    logger.info(f"update: {update.model_dump(mode='json')}")
    except srpc.SRPCResponseError as e:
        logger.error(
            f"failed sending message or processing response on SRPC: {e}",
            exc_info=True,
        )
        raise RuntimeError(
            "failed sending message or processing response on SRPC"
        ) from e
    except Exception as e:
        logger.error(
            f"failed sending message or processing response: {e}",
            exc_info=True,
        )
        raise RuntimeError("failed sending message or processing response") from e

    return output


if __name__ == "__main__":
    asyncio.run(main())
