import argparse
import asyncio
import logging

import slimrpc
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
from slima2a.types.a2a_pb2_slimrpc import add_A2AServiceServicer_to_server


async def main() -> None:
    args = parse_arguments()

    logging.basicConfig(level=args.log_level)

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

    servicer: SRPCHandler | A2AStarletteApplication
    match args.type:
        case "slimrpc":
            servicer = SRPCHandler(agent_card, default_request_handler)

            server = await slimrpc.Server.from_slim_app_config(
                slim_app_config=slimrpc.SLIMAppConfig(
                    identity="agntcy/demo/echo_agent",
                    slim_client_config={
                        "endpoint": "http://localhost:46357",
                        "tls": {
                            "insecure": True,
                        },
                    },
                    shared_secret="secretsecretsecretsecretsecretsecret",
                )
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
        case _:
            raise ValueError(f"Invalid server type: {args.type}")


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()

    parser.add_argument("--type", type=str, required=False, default="slimrpc")

    parser.add_argument(
        "--log-level",
        type=str,
        required=False,
        default="ERROR",
    )

    args = parser.parse_args()

    if args.type not in ["slimrpc", "starlette"]:
        raise ValueError(f"Invalid server type: {args.type}")

    return args


if __name__ == "__main__":
    asyncio.run(main())
