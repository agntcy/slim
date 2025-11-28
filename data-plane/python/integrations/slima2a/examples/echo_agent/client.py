import argparse
import asyncio
import logging
from uuid import uuid4

import httpx
import slimrpc
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

from slima2a.client_transport import (
    ClientConfig,
    SRPCTransport,
    slimrpc_channel_factory,
)

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

    slim_local_app = await slimrpc.common.create_local_app(
        slimrpc.SLIMAppConfig(
            identity="agntcy/demo/client",
            slim_client_config={
                "endpoint": "http://localhost:46357",
                "tls": {
                    "insecure": True,
                },
            },
            shared_secret="secretsecretsecretsecretsecretsecret",
        ),
    )

    client_config = ClientConfig(
        supported_transports=["JSONRPC", "slimrpc"],
        streaming=args.stream,
        httpx_client=httpx_client,
        slimrpc_channel_factory=slimrpc_channel_factory(slim_local_app),
    )
    client_factory = ClientFactory(client_config)

    # mypy: the register API expects a different callable type; safe to ignore here.
    client_factory.register("slimrpc", SRPCTransport.create)  # type: ignore

    agent_card: AgentCard
    match args.type:
        case "slimrpc":
            agent_card = minimal_agent_card("agntcy/demo/echo_agent", ["slimrpc"])
        case "starlette":
            agent_card = await fetch_agent_card(
                resolver=A2ACardResolver(
                    httpx_client=httpx_client,
                    base_url=BASE_URL,
                )
            )
        case _:
            raise ValueError(f"Invalid client type: {args.type}")

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
        default="slimrpc",
    )

    args = parser.parse_args()

    if args.type not in ["slimrpc", "starlette"]:
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
    except slimrpc.SRPCResponseError as e:
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
