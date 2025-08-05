import argparse
import asyncio
import logging
from uuid import uuid4

import httpx
from a2a.client import A2ACardResolver, Client, ClientConfig, ClientFactory
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

    async with httpx.AsyncClient() as httpx_client:
        agent_card = await fetch_agent_card(
            resolver=A2ACardResolver(
                httpx_client=httpx_client,
                base_url=BASE_URL,
            )
        )

        client = ClientFactory(
            config=ClientConfig(
                httpx_client=httpx_client,
                streaming=args.stream,
            ),
        ).create(
            card=agent_card,
        )
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
    return parser.parse_args()


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
    except Exception as e:
        logger.error(
            f"failed sending message or processing response: {e}",
            exc_info=True,
        )
        raise RuntimeError("failed sending message or processing response") from e

    return output


if __name__ == "__main__":
    asyncio.run(main())
