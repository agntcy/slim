import asyncio
import logging
from collections.abc import AsyncIterable

import slim_bindings
from examples.slimrpc.simple.types.example_pb2 import ExampleRequest, ExampleResponse
from examples.slimrpc.simple.types.example_pb2_slimrpc import (
    TestServicer,
    add_TestServicer_to_server,
)
from slim_bindings._slim_bindings.slim_bindings import uniffi_set_event_loop

logger = logging.getLogger(__name__)


class TestService(TestServicer):
    async def ExampleUnaryUnary(
        self,
        request: ExampleRequest,
        context: slim_bindings.Context,
    ) -> ExampleResponse:
        logger.info(f"Received unary-unary request: {request}")

        return ExampleResponse(example_integer=1, example_string="Hello, World!")

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


async def amain() -> None:
    uniffi_set_event_loop(asyncio.get_running_loop())  # type: ignore[arg-type]

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

    # Create local name
    local_name = slim_bindings.Name("agntcy", "grpc", "server")

    # Connect to SLIM
    client_config = slim_bindings.new_insecure_client_config("http://localhost:46357")
    conn_id = await service.connect_async(client_config)

    # Create app with shared secret
    local_app = service.create_app_with_secret(
        local_name, "my_shared_secret_for_testing_purposes_only"
    )

    # Subscribe to local name
    await local_app.subscribe_async(local_name, conn_id)

    # Create server
    server = slim_bindings.Server.new_with_connection(local_app, local_name, conn_id)

    # Add servicer
    add_TestServicer_to_server(TestService(), server)

    # Run server
    logger.info("Server starting...")
    await server.serve_async()


def main() -> None:
    """
    Main entry point for the server.
    """
    logging.basicConfig(level=logging.DEBUG)
    try:
        asyncio.run(amain())
    except KeyboardInterrupt:
        print("Server interrupted by user.")
