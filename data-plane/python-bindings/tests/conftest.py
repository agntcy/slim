import asyncio

import pytest_asyncio

import agp_bindings


@pytest_asyncio.fixture(scope="function")
async def server(request):
    # create new server
    global svc_server
    svc_server = await agp_bindings.create_pyservice("cisco", "default", "server")

    # init tracing
    agp_bindings.init_tracing(log_level="info")

    # run gateway server in background
    await agp_bindings.run_server(
        svc_server,
        {"endpoint": request.param, "tls": {"insecure": True}},
    )

    # wait for the server to start
    await asyncio.sleep(1)

    # return the server
    yield svc_server
