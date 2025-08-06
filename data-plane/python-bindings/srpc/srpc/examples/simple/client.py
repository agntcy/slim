import asyncio
import logging

import srpc
from srpc.examples.simple.types.example_pb2 import ExampleRequest
from srpc.examples.simple.types.example_pb2_srpc import TestStub

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


async def amain():
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
        remote="agntcy/grpc/server"
    )

    # Stubs
    stubs = TestStub(channel)

    # Call method
    request = ExampleRequest(example_integer=1, example_string="hello")
    response = await stubs.ExampleUnaryUnary(request)

    logger.info(f"Response: {response}")

    responses = stubs.ExampleUnaryStream(request)
    async for resp in responses:
        logger.info(f"Stream Response: {resp}")

    async def stream_requests():
        for i in range(5):
            yield ExampleRequest(example_integer=i, example_string=f"Request {i}")

    response = await stubs.ExampleStreamUnary(stream_requests())
    logger.info(f"Stream Unary Response: {response}")

    await asyncio.sleep(1)


def main():
    """
    Main entry point for the server.
    """
    try:
        asyncio.run(amain())
    except KeyboardInterrupt:
        print("Server interrupted by user.")
