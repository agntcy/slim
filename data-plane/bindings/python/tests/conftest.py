# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""Pytest configuration for Slim binding tests.

Provides the 'server' fixture which spins up a Slim service. The fixture
initializes tracing, launches the server asynchronously, waits briefly
for readiness, and yields the underlying Service so tests can
establish sessions, publish messages, or perform connection logic.
"""

import asyncio
import typing

import pytest_asyncio
import slim_bindings


class ServerFixture:
    """Wrapper object for server fixture containing both service and configuration."""

    def __init__(self, service: slim_bindings.Service, endpoint: str):
        self.service = service
        self.endpoint = endpoint
        self.local_service = endpoint is not None

    def get_client_config(self) -> typing.Optional[slim_bindings.ClientConfig]:
        return (
            slim_bindings.new_insecure_client_config("http://" + self.endpoint)
            if self.local_service
            else None
        )


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

    # Initialize global state
    tracing_config = slim_bindings.new_tracing_config()
    runtime_config = slim_bindings.new_runtime_config()
    service_config = slim_bindings.new_service_config()

    tracing_config.log_level = "info"
    slim_bindings.initialize_with_configs(
        tracing_config=tracing_config,
        runtime_config=runtime_config,
        service_config=[service_config],
    )

    # Only start server if endpoint is provided
    if local_service:
        # Create server
        svc_server = slim_bindings.Service("localserver")
        # run slim server in background
        server_config = slim_bindings.new_insecure_server_config(endpoint)
        await svc_server.run_server_async(server_config)
    else:
        svc_server = slim_bindings.get_global_service()

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
