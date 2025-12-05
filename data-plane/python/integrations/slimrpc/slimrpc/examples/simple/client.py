import asyncio
import logging
from collections.abc import AsyncGenerator

import slimrpc
from slimrpc.examples.simple.types.example_pb2 import ExampleRequest
from slimrpc.examples.simple.types.example_pb2_slimrpc import TestStub

logger = logging.getLogger(__name__)


async def amain() -> None:
    channel = await slimrpc.Channel.from_slim_app_config(
        remote="agntcy/grpc/server",
        slim_app_config=slimrpc.SLIMAppConfig(
            identity="agntcy/grpc/client",
            slim_client_config={
                "endpoint": "http://localhost:46357",
                "tls": {
                    "insecure": True,
                },
            },
            enable_opentelemetry=False,
            shared_secret="my_shared_secret_for_testing_purposes_only",
        ),
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

        # task to start and process a stream request
        async def do_stream_request(index: int) -> None:
            responses = stubs.ExampleUnaryStream(request, timeout=2)
            async for resp in responses:
                logger.info(f"Stream {index} Response: {resp}")

        async with asyncio.TaskGroup() as tg:
            tg.create_task(do_stream_request(1))
            tg.create_task(do_stream_request(2))

        async def stream_requests() -> AsyncGenerator[ExampleRequest, None]:
            for i in range(10):
                yield ExampleRequest(example_integer=i, example_string=f"Request {i}")

        response = await stubs.ExampleStreamUnary(stream_requests(), timeout=2)
        logger.info(f"Stream Unary Response: {response}")

        stream = stubs.ExampleStreamStream(stream_requests(), timeout=2)
        async for resp in stream:
            logger.info(f"Stream Stream Response: {resp}")
        logger.info("Stream Stream completed")

    except asyncio.TimeoutError:
        logger.exception("timeout while waiting for response")

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
