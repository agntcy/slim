import asyncio
import os

from dotenv import load_dotenv

load_dotenv()

import slim_bindings
from a2a.server.request_handlers import DefaultRequestHandler
from a2a.server.tasks import InMemoryTaskStore
from a2a.types import AgentCapabilities, AgentCard, AgentSkill
from google.adk.a2a.executor.a2a_agent_executor import A2aAgentExecutor
from google.adk.artifacts.in_memory_artifact_service import InMemoryArtifactService
from google.adk.auth.credential_service.in_memory_credential_service import (
    InMemoryCredentialService,
)
from google.adk.memory.in_memory_memory_service import InMemoryMemoryService
from google.adk.runners import Runner
from google.adk.sessions.in_memory_session_service import InMemorySessionService
from slima2a import setup_slim_client
from slima2a.handler import SRPCHandler
from slima2a.types.a2a_pb2_slimrpc import add_A2AServiceServicer_to_server

from k8s_troubleshooting_agent.agent import root_agent

SLIM_URL = os.getenv("SLIM_URL", "http://localhost:46357")
SLIM_NAMESPACE = os.getenv("SLIM_NAMESPACE", "agntcy")
SLIM_GROUP = os.getenv("SLIM_GROUP", "demo")
SLIM_NAME = os.getenv("SLIM_NAME", "k8s_troubleshooting_agent")
SLIM_SECRET = os.getenv("SLIM_SECRET", "secretsecretsecretsecretsecretsecret")


async def main() -> None:
    agent_card = AgentCard(
        name=root_agent.name,
        description=root_agent.description or "A Kubernetes troubleshooting agent",
        url=f"{SLIM_NAMESPACE}/{SLIM_GROUP}/{SLIM_NAME}",
        version="1.0.0",
        default_input_modes=["text/plain"],
        default_output_modes=["text/plain"],
        capabilities=AgentCapabilities(streaming=True),
        skills=[
            AgentSkill(
                id="k8s_troubleshooting",
                name="Kubernetes Troubleshooting",
                description="Diagnose and resolve issues in Kubernetes clusters",
                tags=["kubernetes", "troubleshooting", "k8s"],
            )
        ],
    )

    runner = Runner(
        app_name=root_agent.name,
        agent=root_agent,
        artifact_service=InMemoryArtifactService(),
        session_service=InMemorySessionService(),
        memory_service=InMemoryMemoryService(),
        credential_service=InMemoryCredentialService(),
    )

    agent_executor = A2aAgentExecutor(runner=runner)

    request_handler = DefaultRequestHandler(
        agent_executor=agent_executor,
        task_store=InMemoryTaskStore(),
    )

    servicer = SRPCHandler(agent_card, request_handler)

    service, local_app, local_name, conn_id = await setup_slim_client(
        namespace=SLIM_NAMESPACE,
        group=SLIM_GROUP,
        name=SLIM_NAME,
        slim_url=SLIM_URL,
        secret=SLIM_SECRET,
    )

    server = slim_bindings.Server.new_with_connection(local_app, local_name, conn_id)
    add_A2AServiceServicer_to_server(servicer, server)

    await server.serve_async()


if __name__ == "__main__":
    asyncio.run(main())
