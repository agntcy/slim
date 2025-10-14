import asyncio
import logging

# Disable a2a telemetry debugging completely
logging.getLogger("a2a.utils.telemetry").setLevel(logging.ERROR)  # type: ignore
logging.getLogger("asyncio").setLevel(logging.ERROR)  # type: ignore

# ruff: noqa: E402
import slimrpc
from a2a.server.request_handlers import DefaultRequestHandler
from a2a.server.tasks import InMemoryTaskStore
from a2a.types import (
    AgentCapabilities,
    AgentCard,
    AgentSkill,
)

from examples.travel_planner_agent.agent_executor import TravelPlannerAgentExecutor
from slima2a.handler import SRPCHandler
from slima2a.types.a2a_pb2_slimrpc import add_A2AServiceServicer_to_server


async def main() -> None:
    skill = AgentSkill(
        id="travel_planner",
        name="travel planner agent",
        description="travel planner",
        tags=["travel planner"],
        examples=["hello", "nice to meet you!"],
    )

    agent_card = AgentCard(
        name="travel planner Agent",
        description="travel planner",
        url="http://localhost:10001/",
        version="1.0.0",
        default_input_modes=["text"],
        default_output_modes=["text"],
        capabilities=AgentCapabilities(streaming=True),
        skills=[skill],
    )

    request_handler = DefaultRequestHandler(
        agent_executor=TravelPlannerAgentExecutor(),
        task_store=InMemoryTaskStore(),
    )

    servicer = SRPCHandler(agent_card, request_handler)
    server = await slimrpc.Server.from_slim_app_config(
        slim_app_config=slimrpc.SLIMAppConfig(
            identity="agntcy/demo/travel_planner_agent",
            slim_client_config={
                "endpoint": "http://localhost:46357",
                "tls": {
                    "insecure": True,
                },
            },
            shared_secret="secret",
        )
    )
    add_A2AServiceServicer_to_server(
        servicer,
        server,
    )

    await server.run()


if __name__ == "__main__":
    asyncio.run(main())
