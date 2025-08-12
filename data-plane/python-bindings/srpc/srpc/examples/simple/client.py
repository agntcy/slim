import asyncio
import logging
from collections.abc import AsyncGenerator

import srpc
from srpc.examples.simple.types.example_pb2 import ExampleRequest
from srpc.examples.simple.types.example_pb2_srpc import TestStub

logger = logging.getLogger(__name__)


async def amain() -> None:
    channel = srpc.Channel(
        local="agntcy/grpc/client",
        slim={
            "endpoint": "http://localhost:46357",
            "tls": {
                "insecure": True,
            },
        },
        enable_opentelemetry=False,
        shared_secret="my_shared_secret",
        remote="agntcy/grpc/server",
    )

    # Stubs
    stubs = TestStub(channel)

    # Call method
    try:
        request = ExampleRequest(example_integer=1, example_string="hello")
        response = await stubs.ExampleUnaryUnary(request, timeout=2)

        logger.info(f"Response: {response}")

        responses = stubs.ExampleUnaryStream(request, timeout=2)
        async for resp in responses:
            logger.info(f"Stream Response: {resp}")

        async def stream_requests() -> AsyncGenerator[ExampleRequest]:
            for i in range(10):
                yield ExampleRequest(example_integer=i, example_string=f"Request {i}")

        response = await stubs.ExampleStreamUnary(stream_requests(), timeout=2)
        logger.info(f"Stream Unary Response: {response}")
    except asyncio.TimeoutError:
        logger.error("timeout while waiting for response")

    await asyncio.sleep(1)


def main() -> None:
    """
    Main entry point for the server.
    """
    logging.basicConfig(level=logging.INFO)
    try:
        asyncio.run(amain())
    except KeyboardInterrupt:
        print("Server interrupted by user.")
