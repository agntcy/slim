# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""Pytest configuration for Slim binding tests.

Provides the 'server' fixture which spins up a Slim service. The fixture
initializes tracing, launches the server asynchronously, waits briefly
for readiness, and yields the underlying Service so tests can
establish sessions, publish messages, or perform connection logic.
"""

import asyncio

import pytest_asyncio
from common import create_slim, create_name, create_server_config

import slim_uniffi_bindings._slim_bindings.slim_bindings as slim_bindings


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
        1. Creates a Service with SharedSecret auth (identity 'server').
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

    name = create_name("agntcy", "default", "server")
    svc_server = create_slim(name, local_service=local_service)

    # Initialize crypto provider (replaces init_tracing in new API)
    slim_bindings.initialize_crypto_provider()

    # Only start server if endpoint is provided
    if endpoint is not None:
        # run slim server in background  
        server_config = create_server_config(endpoint, insecure=True)
        await svc_server.run_server_async(server_config)

        # wait for the server to start
        await asyncio.sleep(1)

    # return the server fixture wrapper
    yield ServerFixture(svc_server, endpoint)
    
    # Teardown: stop server if it was started
    if endpoint is not None:
        try:
            svc_server.stop_server(endpoint)
        except Exception as e:
            # Ignore errors during cleanup
            print(f"Warning: error stopping server {endpoint}: {e}")

