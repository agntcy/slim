import asyncio
import logging
from collections.abc import AsyncIterable

from slimrpc.common import SLIMAppConfig
from slimrpc.context import MessageContext, SessionContext
from slimrpc.examples.simple.types.example_pb2 import ExampleRequest, ExampleResponse
from slimrpc.examples.simple.types.example_pb2_slimrpc import (
    TestServicer,
    add_TestServicer_to_server,
)
from slimrpc.server import Server

logger = logging.getLogger(__name__)


class TestService(TestServicer):
    async def ExampleUnaryUnary(
        self,
        request: ExampleRequest,
        msg_context: MessageContext,
        session_context: SessionContext,
    ) -> ExampleResponse:
        logger.info(f"Received unary-unary request: {request}")

        return ExampleResponse(example_integer=1, example_string="Hello, World!")

    async def ExampleUnaryStream(
        self,
        request: ExampleRequest,
        msg_context: MessageContext,
        session_context: SessionContext,
    ) -> AsyncIterable[ExampleResponse]:
        logger.info(f"Received unary-stream request: {request}")

        # generate async responses stream
        for i in range(5):
            logger.info(f"Sending response {i}")
            yield ExampleResponse(example_integer=i, example_string=f"Response {i}")

    async def ExampleStreamUnary(
        self,
        request_iterator: AsyncIterable[tuple[ExampleRequest, MessageContext]],
        session_context: SessionContext,
    ) -> ExampleResponse:
        logger.info(f"Received stream-unary request: {request_iterator}")

        async for request, msg_ctx in request_iterator:
            _ = msg_ctx  # Unused in this example
            logger.info(f"Received stream-unary request: {request}")

        response = ExampleResponse(
            example_integer=1, example_string="Stream Unary Response"
        )
        return response

    async def ExampleStreamStream(
        self,
        request_iterator: AsyncIterable[tuple[ExampleRequest, MessageContext]],
        session_context: MessageContext,
    ) -> AsyncIterable[ExampleResponse]:
        """Missing associated documentation comment in .proto file."""
        raise NotImplementedError("Method not implemented!")


async def amain() -> None:
    server = await Server.from_slim_app_config(
        slim_app_config=SLIMAppConfig(
            identity="agntcy/grpc/server",
            slim_client_config={
                "endpoint": "http://localhost:46357",
                "tls": {"insecure": True},
            },
            enable_opentelemetry=False,
            shared_secret="my_shared_secret",
        )
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
