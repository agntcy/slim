import asyncio
import logging
from collections.abc import AsyncGenerator
from datetime import timedelta

import slim_bindings
from examples.constants import (
    NAME_NS,
    NAME_ORG,
    SHARED_SECRET,
    SLIM_ADDR,
)
from examples.slimrpc.simple.types.example_pb2 import ExampleRequest
from examples.slimrpc.simple.types.example_pb2_slimrpc import TestStub

logger = logging.getLogger(__name__)


async def amain() -> None:
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

    # Create local and remote names
    local_name = slim_bindings.Name(NAME_ORG, NAME_NS, "client")
    remote_name = slim_bindings.Name(NAME_ORG, NAME_NS, "server")

    # Connect to SLIM
    client_config = slim_bindings.new_insecure_client_config(SLIM_ADDR)
    conn_id = await service.connect_async(client_config)

    # Create app with shared secret
    local_app = service.create_app_with_secret(local_name, SHARED_SECRET)

    # Subscribe to local name
    await local_app.subscribe_async(local_name, conn_id)

    # Create channel
    channel = slim_bindings.Channel.new_with_connection(local_app, remote_name, conn_id)

    # Create stubs
    stubs = TestStub(channel)

    # Call method
    try:
        request = ExampleRequest(example_integer=1, example_string="hello")
        response = await stubs.ExampleUnaryUnary(request, timeout=timedelta(seconds=2))

        logger.info(f"Response: {response}")

        async for resp in stubs.ExampleUnaryStream(
            request, timeout=timedelta(seconds=2)
        ):
            logger.info(f"Stream Response: {resp}")

        async def stream_requests() -> AsyncGenerator[ExampleRequest, None]:
            for i in range(10):
                yield ExampleRequest(example_integer=i, example_string=f"Request {i}")

        response = await stubs.ExampleStreamUnary(
            stream_requests(), timeout=timedelta(seconds=2)
        )
        logger.info(f"Stream Unary Response: {response}")

        async for resp in stubs.ExampleStreamStream(
            stream_requests(), timeout=timedelta(seconds=2)
        ):
            logger.info(f"Stream Stream Response: {resp}")
        logger.info("Stream Stream completed")

        # Close the channel
        await channel.close_async(timeout=None)

    except asyncio.TimeoutError:
        logger.exception("timeout while waiting for response")


def main() -> None:
    """
    Main entry point for the client.
    """
    logging.basicConfig(level=logging.INFO)
    try:
        asyncio.run(amain())
    except KeyboardInterrupt:
        print("Client interrupted by user.")
