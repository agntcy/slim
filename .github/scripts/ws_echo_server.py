# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""Minimal binary-safe WebSocket echo server for the wasm browser tests.

Echoes every frame back to the sender verbatim (text as text, binary as
binary), with no greeting frame, so a protobuf frame sent by
`spawn_transport_tasks` round-trips unchanged. Bound to loopback only; intended
solely for local/CI test use (`WS_ECHO_SERVER_URL`).

Usage: python3 ws_echo_server.py [host] [port]   (defaults 127.0.0.1 9001)
"""

import asyncio
import sys

import websockets


async def _echo(connection):
    async for message in connection:
        # `message` is `str` for text frames and `bytes` for binary frames;
        # sending it back preserves the frame type and payload exactly.
        await connection.send(message)


async def _main(host: str, port: int) -> None:
    async with websockets.serve(_echo, host, port):
        print(f"ws echo server listening on ws://{host}:{port}", flush=True)
        await asyncio.Future()  # run until the process is killed


if __name__ == "__main__":
    # Loopback-only bind by default; never expose this test helper publicly.
    host = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 9001
    try:
        asyncio.run(_main(host, port))
    except KeyboardInterrupt:
        pass
