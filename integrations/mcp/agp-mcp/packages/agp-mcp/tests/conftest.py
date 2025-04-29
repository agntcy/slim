# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging

import agp_bindings
import pytest_asyncio

logger = logging.getLogger(__name__)


@pytest_asyncio.fixture(scope="function")
async def server(request):
    # create new server
    global svc_server
    svc_server = await agp_bindings.create_pyservice("cisco", "default", "server")

    # init tracing
    agp_bindings.init_tracing(
        {"log_level": "info", "opentelemetry": {"enabled": False}}
    )

    # run gateway server in background
    await agp_bindings.run_server(
        svc_server,
        {"endpoint": request.param, "tls": {"insecure": True}},
    )

    # wait for the server to start
    await asyncio.sleep(1)

    # return the server
    yield svc_server
