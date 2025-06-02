# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio

import pytest_asyncio

import slim_bindings


@pytest_asyncio.fixture(scope="function")
async def server(request):
    # create new server
    global svc_server
    svc_server = await slim_bindings.create_pyservice("cisco", "default", "server")

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
