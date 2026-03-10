import asyncio
import logging
import time
from datetime import timedelta

import slim_bindings
from examples.slimrpc.simple.types.example_pb2 import ExampleRequest
from examples.slimrpc.simple.types.example_pb2_slimrpc import TestGroupStub

logger = logging.getLogger(__name__)

# Names of the server instances to broadcast to.
# Start each with: python -m examples.slimrpc.simple.server --instance server1 (or server2)
SERVER_NAMES = [
    slim_bindings.Name("agntcy", "grpc", "server1"),
    slim_bindings.Name("agntcy", "grpc", "server2"),
]


async def run_multicast_unary(stub: TestGroupStub) -> None:
    logger.info("--- multicast unary-unary ---")
    request = ExampleRequest(example_integer=1, example_string="hello")
    async for context, resp in stub.ExampleUnaryUnary(
        request, timeout=timedelta(seconds=5)
    ):
        logger.info(f"  [{context}] {resp}")


async def run_multicast_unary_stream(stub: TestGroupStub) -> None:
    logger.info("--- multicast unary-stream ---")
    request = ExampleRequest(example_integer=1, example_string="hello")
    async for context, resp in stub.ExampleUnaryStream(
        request, timeout=timedelta(seconds=20)
    ):
        logger.info(f"{time.time()}")
        logger.info(f"  [{context}] {resp}")


async def stream_requests():
    for i in range(3):
        yield ExampleRequest(example_integer=i, example_string=f"item {i}")


async def run_multicast_stream_unary(stub: TestGroupStub) -> None:
    logger.info("--- multicast stream-unary ---")
    async for context, resp in stub.ExampleStreamUnary(
        stream_requests(), timeout=timedelta(seconds=5)
    ):
        logger.info(f"  [{context}] {resp}")


async def run_multicast_stream_stream(stub: TestGroupStub) -> None:
    logger.info("--- multicast stream-stream ---")
    async for context, resp in stub.ExampleStreamStream(
        stream_requests(), timeout=timedelta(seconds=5)
    ):
        logger.info(f"  [{context}] {resp}")


async def amain() -> None:
    slim_bindings.uniffi_set_event_loop(asyncio.get_running_loop())  # type: ignore[arg-type]

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

    local_name = slim_bindings.Name("agntcy", "grpc", "client")

    client_config = slim_bindings.new_insecure_client_config("http://localhost:46357")
    conn_id = await service.connect_async(client_config)

    local_app = service.create_app_with_secret(
        local_name, "my_shared_secret_for_testing_purposes_only"
    )
    await local_app.subscribe_async(local_name, conn_id)

    # Group channel targeting all server instances
    channel = slim_bindings.Channel.new_group_with_connection(
        local_app, SERVER_NAMES, conn_id
    )

    stub = TestGroupStub(channel)

    try:
        # await run_multicast_unary(stub)
        # await run_multicast_unary_stream(stub)
        # await run_multicast_stream_unary(stub)
        await run_multicast_stream_stream(stub)
    except slim_bindings.RpcError as e:
        logger.error(f"RPC error: {e}")

    # Close the channel
    await channel.close_async(timeout=None)


def main() -> None:
    logging.basicConfig(level=logging.INFO)
    try:
        asyncio.run(amain())
    except KeyboardInterrupt:
        print("Client interrupted by user.")
