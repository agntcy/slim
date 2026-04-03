# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
import os

logger = logging.getLogger(__name__)

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
from slima2a.handler import SRPCHandler
from slima2a.types.a2a_pb2_slimrpc import add_A2AServiceServicer_to_server

from k8s_troubleshooting_agent.agent import root_agent

# SLIM connection
SLIM_URL = os.getenv("SLIM_URL", "http://localhost:46357")
SLIM_NAMESPACE = os.getenv("SLIM_NAMESPACE", "agntcy")
SLIM_GROUP = os.getenv("SLIM_GROUP", "demo")
SLIM_NAME = os.getenv("SLIM_NAME", "k8s_troubleshooting_agent")

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
    local_name = slim_bindings.Name(SLIM_NAMESPACE, SLIM_GROUP, SLIM_NAME)

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

    local_app, local_name, conn_id = await create_slim_app()

    server = slim_bindings.Server.new_with_connection(local_app, local_name, conn_id)
    add_A2AServiceServicer_to_server(servicer, server)

    await server.serve_async()


if __name__ == "__main__":
    asyncio.run(main())
