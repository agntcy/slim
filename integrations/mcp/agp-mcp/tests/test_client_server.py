# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime
import logging
from collections.abc import AsyncGenerator, Set

import mcp.types as types
import pytest
from mcp import ClientSession
from mcp.server.lowlevel import Server

from agp_mcp import AGPClient, AGPServer

# Configure logging
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

# Test configuration
TEST_ORG = "cisco"
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
    app = Server("example-server")

    @app.list_tools()
    async def list_tools() -> list[types.Tool]:
        return [example_tool]

    return app


class SessionManager:
    """Manages multiple server sessions."""

    def __init__(self):
        self.active_sessions: Set[asyncio.Task] = set()
        self._lock = asyncio.Lock()

    async def add_session(self, task: asyncio.Task) -> None:
        """Add a new session task to the active sessions."""
        async with self._lock:
            self.active_sessions.add(task)
            task.add_done_callback(self._remove_session)

    def _remove_session(self, task: asyncio.Task) -> None:
        """Remove a completed session task."""
        self.active_sessions.discard(task)

    async def cancel_all(self) -> None:
        """Cancel all active sessions."""
        async with self._lock:
            for task in self.active_sessions:
                if not task.done():
                    task.cancel()
            self.active_sessions.clear()


async def handle_server_session(
    agp_server: AGPServer, app: Server, session_manager: SessionManager
) -> AsyncGenerator[None, None]:
    """Handle incoming server sessions.

    Args:
        agp_server: The AGP server instance
        app: The MCP server application
        session_manager: Session manager to track active sessions
    """

    try:
        async for session in agp_server:
            assert session is not None, "Received null session"
            logger.info(f"New session received: {session}")

            async def run_session():
                try:
                    async with agp_server.new_session(session) as streams:
                        await app.run(
                            streams[0], streams[1], app.create_initialization_options()
                        )
                except Exception as e:
                    logger.error(f"Error in session {session}: {e}")
                    raise

            # Create and track the session task
            session_task = asyncio.create_task(run_session())
            await session_manager.add_session(session_task)

    except Exception as e:
        logger.error(f"Error in handle_server_session: {e}")
        raise
    finally:
        await session_manager.cancel_all()


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12345"], indirect=True)
async def test_mcp_client_server_connection(server, mcp_app):
    """Test basic MCP client-server connection and initialization."""
    config = get_test_config(12345)
    logger.info(f"Starting AGP MCP server on {server}...")

    session_manager = SessionManager()

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
        handler_task = asyncio.create_task(
            handle_server_session(agp_server, mcp_app, session_manager)
        )

        try:
            async with agp_client.new_session() as streams:
                async with ClientSession(streams[0], streams[1]) as mcp_session:
                    # Test session initialization
                    await mcp_session.initialize()
                    logger.info(
                        f"Client session initialized at {datetime.datetime.now().isoformat()}"
                    )

                    # Test tool listing
                    tools = await mcp_session.list_tools()
                    assert tools is not None, "Failed to list tools"
                    # assert len(tools) == 1, "Expected exactly one tool"
                    # assert tools[0].name == "example", "Tool name mismatch"

                    logger.info(f"Successfully retrieved tools: {tools}")
        except Exception as e:
            logger.error(f"Error during client-server interaction: {e}")
            raise
        finally:
            # Cleanup
            handler_task.cancel()
            await session_manager.cancel_all()
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

    session_manager = SessionManager()

    async with AGPServer(config, TEST_ORG, TEST_NS, TEST_MCP_SERVER) as agp_server:
        handler_task = asyncio.create_task(
            handle_server_session(agp_server, mcp_app, session_manager)
        )

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
                async with client1.new_session() as streams:
                    async with ClientSession(streams[0], streams[1]) as session1:
                        await session1.initialize()
                        tools1 = await session1.list_tools()
                        assert tools1 is not None
                        logger.info("First session completed successfully")

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
                async with client2.new_session() as streams:
                    async with ClientSession(streams[0], streams[1]) as session2:
                        logger.info("Second session initializing")
                        await session2.initialize()
                        logger.info("Second session initialized")
                        tools2 = await session2.list_tools()
                        assert tools2 is not None
                        logger.info("Second session completed successfully")
        finally:
            # Cleanup
            handler_task.cancel()
            await session_manager.cancel_all()
            try:
                await handler_task
            except asyncio.CancelledError:
                pass
