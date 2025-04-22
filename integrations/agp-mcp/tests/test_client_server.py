# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime

import mcp.types as types
import pytest
from mcp import ClientSession
from mcp.server.lowlevel import Server

from agp_mcp import AGPClient, AGPServer

# fake tools


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12345"], indirect=True)
async def test_mcp_client_server(server):
    org = "cisco"
    ns = "default"
    mcp_server = "mcp1"

    print(f"Starting AGP MCP server on {server}...")

    # create mcp server with AGP transport
    config = {
        "endpoint": "http://127.0.0.1:12345",
        "tls": {
            "insecure": True,
        },
    }

    # Create MCP app
    app = Server("example-server")

    # register some tools
    @app.list_tools()
    async def list_tools() -> list[types.Tool]:
        return [
            types.Tool(
                name="example",
                description="The most exemplar tool of the tools",
                inputSchema={
                    "type": "object",
                    "required": ["url"],
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "example URL input parameter",
                        }
                    },
                },
            )
        ]

    async with (
        AGPServer(config, org, ns, mcp_server) as agp_server,
        AGPClient(config, org, ns, "client1", org, ns, mcp_server) as agp_client,
    ):
        # Check if the server is running
        assert agp_server.gateway is not None

        async def handle_session():
            # Wait for a new session to be created
            async for session in agp_server:
                assert session is not None

                # Create server session
                async with agp_server.new_session(session) as streams:
                    await app.run(
                        streams[0], streams[1], app.create_initialization_options()
                    )

        # Start the session handler
        session_task = asyncio.create_task(handle_session())

        async with agp_client.new_session() as streams:
            async with ClientSession(streams[0], streams[1]) as mcp_session:
                try:
                    await mcp_session.initialize()

                    print(
                        f"Client session initialized at {datetime.datetime.now().isoformat()}"
                    )

                    tools = await mcp_session.list_tools()
                    assert tools is not None
                    print(tools)
                except Exception as e:
                    print(f"Error initializing session: {e}")
                    raise e

                # We are good to exit
                session_task.cancel()

        try:
            await session_task
        except asyncio.CancelledError:
            pass
