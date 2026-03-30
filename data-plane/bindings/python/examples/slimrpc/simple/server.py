import argparse
import asyncio
import logging
from collections.abc import AsyncIterable

import slim_bindings
from examples.constants import (
    NAME_NS,
    NAME_ORG,
    SHARED_SECRET,
    SLIM_ADDR,
)
from examples.slimrpc.simple.types.example_pb2 import ExampleRequest, ExampleResponse
from examples.slimrpc.simple.types.example_pb2_slimrpc import (
    TestServicer,
    add_TestServicer_to_server,
)

logger = logging.getLogger(__name__)


class TestService(TestServicer):
    async def ExampleUnaryUnary(
        self,
        request: ExampleRequest,
        context: slim_bindings.Context,
    ) -> ExampleResponse:
        logger.info(f"Received unary-unary request: {request}")
        return ExampleResponse(example_integer=1, example_string="Hello, World!")

        # If you need to return a specific error
        # raise slim_bindings.RpcError.Rpc(
        #     code=slim_bindings.RpcCode.UNIMPLEMENTED,
        #     message="not implemented (yet)",
        #     details=None
        # )
        #
        # If you need to return a simple error
        # raise RuntimeError("the error")
        # This will be trated with an "INTERNAL" error code

    async def ExampleUnaryStream(
        self,
        request: ExampleRequest,
        context: slim_bindings.Context,
    ) -> AsyncIterable[ExampleResponse]:
        logger.info(f"Received unary-stream request: {request}")

        # generate async responses stream
        for i in range(5):
            logger.info(f"Sending response {i}")
            yield ExampleResponse(example_integer=i, example_string=f"Response {i}")

    async def ExampleStreamUnary(
        self,
        request_iterator: AsyncIterable[ExampleRequest],
        context: slim_bindings.Context,
    ) -> ExampleResponse:
        logger.info("Received stream-unary request")

        received_strs = []
        async for request in request_iterator:
            logger.info(f"Received request: {request}")
            received_strs.append(request.example_string)

        response = ExampleResponse(
            example_integer=len(received_strs),
            example_string="Saw: " + ", ".join(received_strs),
        )
        return response

    async def ExampleStreamStream(
        self,
        request_iterator: AsyncIterable[ExampleRequest],
        context: slim_bindings.Context,
    ) -> AsyncIterable[ExampleResponse]:
        logger.info("Received stream-stream request")
        async for request in request_iterator:
            logger.info(f"Echoing back request: {request}")
            yield ExampleResponse(
                example_integer=request.example_integer * 100,
                example_string=f"Echo: {request.example_string}",
            )


async def amain(instance: str, server: str) -> None:
    slim_bindings.uniffi_set_event_loop(asyncio.get_running_loop())  # type: ignore[arg-type]

    # Initialize service
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

    # Create local name (instance allows running multiple servers, e.g. server1, server2)
    local_name = slim_bindings.Name(NAME_ORG, NAME_NS, instance)

    # Connect to SLIM
    client_config = slim_bindings.new_insecure_client_config(server)
    conn_id = await service.connect_async(client_config)

    # Create app with shared secret
    local_app = service.create_app_with_secret(local_name, SHARED_SECRET)

    # Subscribe to local name
    await local_app.subscribe_async(local_name, conn_id)

    # Create server
    rpc_server = slim_bindings.Server.new_with_connection(
        local_app, local_name, conn_id
    )

    # Add servicer
    add_TestServicer_to_server(TestService(), rpc_server)

    # Run server
    print("SLIM_RPC_SERVER_READY", flush=True)
    logger.info(f"Server '{instance}' starting...")
    await rpc_server.serve_async()


def main() -> None:
    """
    Main entry point for the server.
    """
    parser = argparse.ArgumentParser(description="SlimRPC example server")
    parser.add_argument(
        "--instance",
        default="server",
        help="Instance name used as the SLIM app name (default: server). "
        "Use server1/server2 when running alongside client_group.py.",
    )
    parser.add_argument(
        "--server",
        default=SLIM_ADDR,
        help="SLIM server endpoint (default: from SLIM_ADDR env var or http://localhost:46357)",
    )
    args = parser.parse_args()

    logging.basicConfig(level=logging.DEBUG)
    try:
        asyncio.run(amain(args.instance, args.server))
    except KeyboardInterrupt:
        print("Server interrupted by user.")
