# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""Pytest configuration for Slim binding tests.

Provides the 'server' fixture which spins up a Slim service. The fixture
initializes tracing, launches the server asynchronously, waits briefly
for readiness, and yields the underlying PyService so tests can
establish sessions, publish messages, or perform connection logic.
"""

import asyncio

import pytest_asyncio

import slim_bindings


class ServerFixture:
    """Wrapper object for server fixture containing both service and configuration."""

    def __init__(self, service, endpoint):
        self.service = service
        self.endpoint = endpoint
        self.local_service = endpoint is not None


@pytest_asyncio.fixture(scope="function")
async def server(request):
    """Async pytest fixture that launches a Slim service instance acting as a server.

    Parametrization:
        request.param: Endpoint string or None
            - String: Endpoint to bind server (e.g. "127.0.0.1:12345") - creates local service
            - None: No server created - creates global service

    Behavior:
        1. Creates a PyService with SharedSecret auth (identity 'server').
           This is not used here, as we use this SLIM instance only for packet forwarding.
        2. Initializes tracing (log_level=info) once.
        3. Starts the server with the provided endpoint (non-blocking) if endpoint provided.
        4. Waits briefly (1s) to ensure the server socket is listening (if server started).
        5. Yields a ServerFixture object containing service and configuration.
        6. Cleanup is handled automatically by the event loop / service drop.
    """

    # Get endpoint parameter
    endpoint = request.param
    local_service = endpoint is not None

    name = slim_bindings.PyName("agntcy", "default", "server")
    provider = slim_bindings.PyIdentityProvider.SharedSecret(
        identity="server", shared_secret="secret"
    )
    verifier = slim_bindings.PyIdentityVerifier.SharedSecret(
        identity="server", shared_secret="secret"
    )

    svc_server = await slim_bindings.create_pyservice(
        name, provider, verifier, local_service=local_service
    )

    # init tracing
    await slim_bindings.init_tracing({"log_level": "info"})

    # Only start server if endpoint is provided
    if endpoint is not None:
        # run slim server in background
        await slim_bindings.run_server(
            svc_server,
            {"endpoint": endpoint, "tls": {"insecure": True}},
        )

        # wait for the server to start
        await asyncio.sleep(1)

    # return the server fixture wrapper
    yield ServerFixture(svc_server, endpoint)
