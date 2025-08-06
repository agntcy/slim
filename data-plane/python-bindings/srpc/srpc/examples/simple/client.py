import asyncio
import logging

from srpc.client import SRPCChannel
from srpc.grpc.example_pb2 import ExampleRequest
from srpc.grpc.example_pb2_srpc import TestStub

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


def create_channel(
    local: str,
    slim: dict,
    enable_opentelemetry: bool = False,
    shared_secret: str | None = None,
):
    """
    Create a new SRPC channel instance.
    """
    channel = SRPCChannel(
        local=local,
        slim=slim,
        enable_opentelemetry=enable_opentelemetry,
        shared_secret=shared_secret,
    )

    return channel


async def amain():
    channel = create_channel(
        local="agntcy/grpc/client",
        slim={
            "endpoint": "http://localhost:46357",
            "tls": {
                "insecure": True,
            },
        },
        enable_opentelemetry=False,
        shared_secret="my_shared_secret",
    )

    # Connect channel
    await channel.connect("agntcy/grpc/server")

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
