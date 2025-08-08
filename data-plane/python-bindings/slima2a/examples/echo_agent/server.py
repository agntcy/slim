import argparse
import asyncio

import srpc
import uvicorn
from a2a.server.apps import A2AStarletteApplication
from a2a.server.request_handlers import DefaultRequestHandler
from a2a.server.tasks import InMemoryTaskStore
from a2a.types import (
    AgentCapabilities,
    AgentCard,
    AgentSkill,
)

from examples.echo_agent.echo_agent_executor import EchoAgentExecutor
from slima2a.handler import SRPCHandler
from slima2a.types.a2a_pb2_srpc import add_A2AServiceServicer_to_server


async def main() -> None:
    args = parse_arguments()

    skill = AgentSkill(
        id="echo",
        name="echo",
        description="returns the received prompt",
        tags=["echo"],
        examples=["hi", "hello", "how are you"],
    )

    agent_card = AgentCard(
        name="Echo Agent",
        description="Just a simple echo agent that returns the received prompt",
        url="http://localhost:9999/",
        version="1.0.0",
        default_input_modes=["text"],
        default_output_modes=["text"],
        capabilities=AgentCapabilities(streaming=True),
        skills=[skill],
    )

    agent_executor = EchoAgentExecutor()
    task_store = InMemoryTaskStore()
    default_request_handler = DefaultRequestHandler(
        agent_executor=agent_executor,
        task_store=task_store,
    )

    servicer = None
    match args.type:
        case "srpc":
            servicer = SRPCHandler(agent_card, default_request_handler)

            server = srpc.Server(
                local="agntcy/demo/echo_agent",
                slim={
                    "endpoint": "http://localhost:46357",
                    "tls": {
                        "insecure": True,
                    },
                },
                shared_secret="secret",
            )
            add_A2AServiceServicer_to_server(
                servicer,
                server,
            )

            await server.run()
        case "starlette":
            servicer = A2AStarletteApplication(
                agent_card=agent_card,
                http_handler=default_request_handler,
            )

            uvicorn.run(servicer.build(), host="0.0.0.0", port=9999)


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()

    parser.add_argument("--type", type=str, required=False, default="srpc")

    args = parser.parse_args()

    if args.type not in ["srpc", "starlette"]:
        raise ValueError(f"Invalid server type: {args.type}")

    return args


if __name__ == "__main__":
    asyncio.run(main())
