import asyncio
import logging
from uuid import uuid4

# Disable a2a telemetry debugging before any a2a imports
logging.getLogger("a2a.utils.telemetry").setLevel(logging.ERROR)
logging.getLogger("asyncio").setLevel(logging.ERROR)

# ruff: noqa: E402
import httpx
import slimrpc
from a2a.client import (
    Client,
    ClientFactory,
    minimal_agent_card,
)
from a2a.types import (
    Message,
    Part,
    Role,
    TextPart,
)

from slima2a.client_transport import (
    ClientConfig,
    SRPCTransport,
    slimrpc_channel_factory,
)


def print_welcome_message() -> None:
    print("Welcome to the generic A2A client!")
    print("Please enter your query (type 'exit' to quit):")


def get_user_query() -> str:
    return input("\n> ")


async def interact_with_server(client: Client) -> None:
    while True:
        user_input = get_user_query()
        if user_input.lower() == "exit":
            print("bye!~")
            break

        request_id = str(uuid4())
        request = Message(
            role=Role.user,
            message_id=request_id,
            parts=[Part(root=TextPart(text=user_input))],
        )

        output = ""
        try:
            async for response in client.send_message(request=request):
                if isinstance(response, Message):
                    for part in response.parts:
                        if isinstance(part.root, TextPart):
                            output += part.root.text
                else:
                    task, _ = response

                    if task.status.state == "completed" and task.artifacts:
                        for artifact in task.artifacts:
                            for part in artifact.parts:
                                if isinstance(part.root, TextPart):
                                    output += part.root.text

        except Exception as e:
            raise RuntimeError("failed sending message or processing response") from e

        print(output, end="", flush=True)
        await asyncio.sleep(0.1)


async def main() -> None:
    print_welcome_message()

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
            shared_secret="secret",
        )
    )

    client_config = ClientConfig(
        supported_transports=["JSONRPC", "slimrpc"],
        streaming=True,
        httpx_client=httpx_client,
        slimrpc_channel_factory=slimrpc_channel_factory(slim_local_app),
    )
    client_factory = ClientFactory(client_config)

    # mypy: the register API expects a different callable type; safe to ignore here.
    client_factory.register("slimrpc", SRPCTransport.create)  # type: ignore
    agent_card = minimal_agent_card("agntcy/demo/travel_planner_agent", ["slimrpc"])
    client = client_factory.create(card=agent_card)

    await interact_with_server(client)


if __name__ == "__main__":
    asyncio.run(main())
