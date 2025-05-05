# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime
import logging

import mcp.types as types
import pytest
from mcp.server.lowlevel import Server

from agp_mcp import AGPClient, AGPServer

# Configure logging
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

# Test configuration
TEST_ORG = "org"
TEST_NS = "default"
TEST_MCP_SERVER = "mcp1"
TEST_CLIENT_ID = "client1"


def get_test_config(port: int) -> dict:
    """Create test configuration with specified port."""
    return {
        "endpoint": f"http://127.0.0.1:{port}",
        "tls": {
            "insecure": True,
        },
    }


async def handle_sessions(mcp_app: Server, agp_server: AGPServer):
    """Handle a session with the MCP server.

    Args:
        mcp_app: The MCP server application
        agp_server: The AGP server instance

    Raises:
        Exception: If there is an error handling sessions
    """
    tasks = set()

    try:
        async for new_session in agp_server:
            # Create a new task for this session
            async def handle_session(session):
                try:
                    async with agp_server.new_streams(session) as streams:
                        await mcp_app.run(
                            streams[0],
                            streams[1],
                            mcp_app.create_initialization_options(),
                        )
                except Exception:
                    logger.error(
                        f"Error handling session {session.id}",
                        extra={"session_id": session.id},
                        exc_info=True,
                    )
                    raise

            task = asyncio.create_task(handle_session(new_session))
            task.add_done_callback(
                lambda t: tasks.discard(t)
            )  # Remove task from set when done
            tasks.add(task)

            # Log new session
            logger.info(
                "New session started",
                extra={"session_id": new_session.id, "active_sessions": len(tasks)},
            )
    except Exception:
        logger.error("Error in session handler", exc_info=True)
        raise
    finally:
        # Cancel all remaining tasks
        for task in tasks:
            if not task.done():
                task.cancel()

        # Wait for all tasks to complete
        if tasks:
            try:
                await asyncio.gather(*tasks, return_exceptions=True)
            except Exception:
                logger.error("Error during task cleanup", exc_info=True)
                raise


@pytest.fixture
def example_tool() -> types.Tool:
    """Create an example tool for testing."""
    return types.Tool(
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


@pytest.fixture
def mcp_app(example_tool: types.Tool) -> Server:
    """Create and configure an MCP server application."""
    app: Server = Server("example-server")

    @app.list_tools()
    async def list_tools() -> list[types.Tool]:
        return [example_tool]

    return app


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12345"], indirect=True)
async def test_mcp_client_server_connection(server, mcp_app):
    """Test basic MCP client-server connection and initialization."""
    config = get_test_config(12345)
    logger.info(f"Starting AGP MCP server on {server}...")

    async with (
        AGPServer(config, TEST_ORG, TEST_NS, TEST_MCP_SERVER) as agp_server,
        AGPClient(
            config,
            TEST_ORG,
            TEST_NS,
            TEST_CLIENT_ID,
            TEST_ORG,
            TEST_NS,
            TEST_MCP_SERVER,
        ) as agp_client,
    ):
        # Verify server is running
        assert agp_server.gateway is not None, "Server gateway not initialized"

        # Start session handler
        handler_task = asyncio.create_task(handle_sessions(mcp_app, agp_server))

        try:
            async with agp_client.to_mcp_session() as mcp_session:
                # Test session initialization
                await mcp_session.initialize()
                logger.info(
                    f"Client session initialized at {datetime.datetime.now().isoformat()}"
                )

                # Test tool listing
                tools = await mcp_session.list_tools()
                assert tools is not None, "Failed to list tools"

                logger.info(f"Successfully retrieved tools: {tools}")
        except Exception as e:
            logger.error(f"Error during client-server interaction: {e}")
            raise
        finally:
            # Cleanup
            handler_task.cancel()
            try:
                await handler_task
            except asyncio.CancelledError:
                pass


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12346"], indirect=True)
async def test_mcp_client_server_reconnection(server, mcp_app):
    """Test client reconnection to server."""
    config = get_test_config(12346)
    logger.info("Testing client reconnection...")

    async with AGPServer(config, TEST_ORG, TEST_NS, TEST_MCP_SERVER) as agp_server:
        handler_task = asyncio.create_task(handle_sessions(mcp_app, agp_server))

        try:
            # First connection
            async with AGPClient(
                config,
                TEST_ORG,
                TEST_NS,
                TEST_CLIENT_ID,
                TEST_ORG,
                TEST_NS,
                TEST_MCP_SERVER,
            ) as client1:
                async with client1.to_mcp_session() as mcp_session:
                    logger.info("First session initialized")
                    await mcp_session.initialize()
                    logger.info("First session completed successfully")

                    # Test tool listing
                    tools = await mcp_session.list_tools()
                    assert tools is not None, "Failed to list tools"
                    logger.info(f"Successfully retrieved tools: {tools}")

            logger.info("Second connection")

            # Second connection with same client ID
            async with AGPClient(
                config,
                TEST_ORG,
                TEST_NS,
                TEST_CLIENT_ID,
                TEST_ORG,
                TEST_NS,
                TEST_MCP_SERVER,
            ) as client2:
                async with client2.to_mcp_session() as mcp_session:
                    logger.info("First session initialized")
                    await mcp_session.initialize()
                    logger.info("First session completed successfully")

                    # Test tool listing
                    tools = await mcp_session.list_tools()
                    assert tools is not None, "Failed to list tools"
                    logger.info(f"Successfully retrieved tools: {tools}")

            # Concurrent connection
            async with (
                AGPClient(
                    config,
                    TEST_ORG,
                    TEST_NS,
                    TEST_CLIENT_ID,
                    TEST_ORG,
                    TEST_NS,
                    TEST_MCP_SERVER,
                ) as client3,
                AGPClient(
                    config,
                    TEST_ORG,
                    TEST_NS,
                    TEST_CLIENT_ID,
                    TEST_ORG,
                    TEST_NS,
                    TEST_MCP_SERVER,
                ) as client4,
                AGPClient(
                    config,
                    TEST_ORG,
                    TEST_NS,
                    TEST_CLIENT_ID,
                    TEST_ORG,
                    TEST_NS,
                    TEST_MCP_SERVER,
                ) as client5,
            ):
                async with (
                    client3.to_mcp_session() as mcp_session1,
                    client4.to_mcp_session() as mcp_session2,
                    client5.to_mcp_session() as mcp_session3,
                ):
                    logger.info("Concurrent sessions initialized")
                    await mcp_session1.initialize()
                    await mcp_session2.initialize()
                    await mcp_session3.initialize()
                    logger.info("Concurrent sessions completed successfully")

                    # Test tool listing
                    tools1 = await mcp_session1.list_tools()
                    assert tools1 is not None, "Failed to list tools"
                    logger.info(f"Successfully retrieved tools: {tools1}")

                    tools2 = await mcp_session2.list_tools()
                    assert tools2 is not None, "Failed to list tools"
                    logger.info(f"Successfully retrieved tools: {tools2}")

                    tools3 = await mcp_session3.list_tools()
                    assert tools3 is not None, "Failed to list tools"
                    logger.info(f"Successfully retrieved tools: {tools3}")

        finally:
            # Cleanup
            handler_task.cancel()
            try:
                await handler_task
            except asyncio.CancelledError:
                pass
