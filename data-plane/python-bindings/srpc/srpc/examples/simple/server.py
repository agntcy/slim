import asyncio
import logging
from collections.abc import AsyncIterable

from srpc.context import Context
from srpc.examples.simple.types.example_pb2 import ExampleRequest, ExampleResponse
from srpc.examples.simple.types.example_pb2_srpc import (
    TestServicer,
    add_TestServicer_to_server,
)
from srpc.server import Server

logger = logging.getLogger(__name__)


class TestService(TestServicer):
    async def ExampleUnaryUnary(
        self, request: ExampleRequest, context: Context
    ) -> ExampleResponse:
        logger.info(f"Received unary-unary request: {request}")

        return ExampleResponse(example_integer=1, example_string="Hello, World!")

    async def ExampleUnaryStream(
        self, request: ExampleRequest, context: Context
    ) -> AsyncIterable[ExampleResponse]:
        logger.info(f"Received unary-stream request: {request}")

        # generate async responses stream
        for i in range(5):
            logger.info(f"Sending response {i}")
            yield ExampleResponse(example_integer=i, example_string=f"Response {i}")

    async def ExampleStreamUnary(
        self, request_iterator: AsyncIterable[ExampleRequest], context: Context
    ) -> ExampleResponse:
        logger.info(f"Received stream-unary request: {request_iterator}")

        async for request in request_iterator:
            logger.info(f"Received stream-unary request: {request}")
        response = ExampleResponse(
            example_integer=1, example_string="Stream Unary Response"
        )
        return response

    async def ExampleStreamStream(
        self, request_iterator: AsyncIterable[ExampleRequest], context: Context
    ) -> AsyncIterable[ExampleResponse]:
        """Missing associated documentation comment in .proto file."""
        raise NotImplementedError("Method not implemented!")


def create_server(
    local: str,
    slim: dict,
    enable_opentelemetry: bool = False,
    shared_secret: str = "",
) -> Server:
    """
    Create a new SRPC server instance.
    """
    server = Server(
        local=local,
        slim=slim,
        enable_opentelemetry=enable_opentelemetry,
        shared_secret=shared_secret,
    )

    return server


async def amain() -> None:
    server = create_server(
        local="agntcy/grpc/server",
        slim={
            "endpoint": "http://localhost:46357",
            "tls": {
                "insecure": True,
            },
        },
        enable_opentelemetry=False,
        shared_secret="my_shared_secret",
    )

    # Create RPCs
    add_TestServicer_to_server(
        TestService(),
        server,
    )

    await server.run()


def main() -> None:
    """
    Main entry point for the server.
    """
    logging.basicConfig(level=logging.DEBUG)
    try:
        asyncio.run(amain())
    except KeyboardInterrupt:
        print("Server interrupted by user.")
