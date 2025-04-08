import asyncio

import pytest_asyncio

import agp_bindings


@pytest_asyncio.fixture(scope="function")
async def server(request):
    # create new server
    global svc_server
    svc_server = await agp_bindings.create_pyservice("cisco", "default", "server")

    # configure it
    svc_server.configure(agp_bindings.GatewayConfig(endpoint=request.param, insecure=True))

    # init tracing
    agp_bindings.init_tracing(log_level="info")

    # run gateway server in background
    await agp_bindings.serve(svc_server)

    # wait for the server to start
    await asyncio.sleep(1)

    # return the server
    yield svc_server
