import argparse
import asyncio
import logging
import time
from datetime import timedelta

import slim_bindings
from examples.constants import (
    NAME_NS,
    NAME_ORG,
    SHARED_SECRET,
    SLIM_ADDR,
)
from examples.slimrpc.simple.types.example_pb2 import ExampleRequest
from examples.slimrpc.simple.types.example_pb2_slimrpc import TestGroupStub

logger = logging.getLogger(__name__)


async def run_multicast_unary(stub: TestGroupStub) -> None:
    logger.info("=== Multicast Unary-Unary ===")
    request = ExampleRequest(example_integer=1, example_string="hello")
    async for context, resp in stub.ExampleUnaryUnary(
        request, timeout=timedelta(seconds=5)
    ):
        logger.info(f"  [{context}] {resp}")


async def run_multicast_unary_stream(stub: TestGroupStub) -> None:
    logger.info("=== Multicast Unary-Stream ===")
    request = ExampleRequest(example_integer=1, example_string="hello")
    async for context, resp in stub.ExampleUnaryStream(
        request, timeout=timedelta(seconds=5)
    ):
        logger.info(f"{time.time()}")
        logger.info(f"  [{context}] {resp}")


async def stream_requests():
    for i in range(3):
        yield ExampleRequest(example_integer=i, example_string=f"item {i}")


async def run_multicast_stream_unary(stub: TestGroupStub) -> None:
    logger.info("=== Multicast Stream-Unary ===")
    async for context, resp in stub.ExampleStreamUnary(
        stream_requests(), timeout=timedelta(seconds=5)
    ):
        logger.info(f"  [{context}] {resp}")


async def run_multicast_stream_stream(stub: TestGroupStub) -> None:
    logger.info("=== Multicast Stream-Stream ===")
    async for context, resp in stub.ExampleStreamStream(
        stream_requests(), timeout=timedelta(seconds=5)
    ):
        logger.info(f"  [{context}] {resp}")


async def amain(server: str, servers_str: str) -> None:
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

    local_name = slim_bindings.Name(NAME_ORG, NAME_NS, "client")

    server_names = [
        slim_bindings.Name(NAME_ORG, NAME_NS, s.strip()) for s in servers_str.split(",")
    ]

    client_config = slim_bindings.new_insecure_client_config(server)
    conn_id = await service.connect_async(client_config)

    local_app = service.create_app_with_secret(local_name, SHARED_SECRET)
    await local_app.subscribe_async(local_name, conn_id)

    # Group channel targeting all server instances
    channel = slim_bindings.Channel.new_group_with_connection(
        local_app, server_names, conn_id
    )

    stub = TestGroupStub(channel)

    print("SLIM_RPC_GROUP_CLIENT_STARTED", flush=True)

    try:
        await run_multicast_unary(stub)
    except slim_bindings.RpcError as e:
        logger.error(f"RPC error in multicast unary-unary: {e}")
        raise

    try:
        await run_multicast_unary_stream(stub)
    except slim_bindings.RpcError as e:
        logger.error(f"RPC error in multicast unary-stream: {e}")
        raise

    try:
        await run_multicast_stream_unary(stub)
    except slim_bindings.RpcError as e:
        logger.error(f"RPC error in multicast stream-unary: {e}")
        raise

    try:
        await run_multicast_stream_stream(stub)
    except slim_bindings.RpcError as e:
        logger.error(f"RPC error in multicast stream-stream: {e}")
        raise

    print("SLIM_RPC_GROUP_CLIENT_DONE", flush=True)

    await asyncio.sleep(1)

    # Close the channel
    await channel.close_async(timeout=None)


def main() -> None:
    parser = argparse.ArgumentParser(description="SlimRPC example group client")
    parser.add_argument(
        "--server",
        default=SLIM_ADDR,
        help="SLIM server endpoint (default: from SLIM_ADDR env var or http://localhost:46357)",
    )
    parser.add_argument(
        "--servers",
        default="server1,server2",
        help="Comma-separated server instance names",
    )
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO)
    try:
        asyncio.run(amain(args.server, args.servers))
    except KeyboardInterrupt:
        print("Client interrupted by user.")
