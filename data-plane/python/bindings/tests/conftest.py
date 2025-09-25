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


@pytest_asyncio.fixture(scope="function")
async def server(request):
    """Async pytest fixture that launches a Slim service instance acting as a server.

    Parametrization:
        request.param: Endpoint string passed via @pytest.mark.parametrize to specify
            the listening endpoint (e.g. "127.0.0.1:12345"). TLS is set
            to insecure=True for test convenience.

    Behavior:
        1. Creates a PyService with SharedSecret auth (identity 'server').
           This is not used here, as we use this SLIM instance only for packet forwarding.
        2. Initializes tracing (log_level=info) once per fixture invocation.
        3. Starts the server with the provided endpoint (non-blocking).
        4. Waits briefly (1s) to ensure the server socket is listening.
        5. Yields the underlying service handle for test use.
        6. Cleanup is handled automatically by the event loop / service drop.
    """
    # create new server
    global svc_server

    name = slim_bindings.PyName("agntcy", "default", "server")
    provider = slim_bindings.PyIdentityProvider.SharedSecret(
        identity="server", shared_secret="secret"
    )
    verifier = slim_bindings.PyIdentityVerifier.SharedSecret(
        identity="server", shared_secret="secret"
    )

    svc_server = await slim_bindings.create_pyservice(name, provider, verifier)

    # init tracing
    await slim_bindings.init_tracing({"log_level": "info"})

    # run slim server in background
    await slim_bindings.run_server(
        svc_server,
        {"endpoint": request.param, "tls": {"insecure": True}},
    )

    # wait for the server to start
    await asyncio.sleep(1)

    # return the server
    yield svc_server
