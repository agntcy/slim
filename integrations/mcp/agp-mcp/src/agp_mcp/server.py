# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import logging
import asyncio
from collections.abc import Set, AsyncGenerator

import agp_bindings
from mcp.server.lowlevel import Server

from agp_mcp.common import AGPBase

logger = logging.getLogger(__name__)


class AGPServer(AGPBase):
    def __init__(
        self,
        config: dict,
        local_organization: str,
        local_namespace: str,
        local_agent: str,
    ):
        """
        AGP transport Server for MCP (Model Context Protocol) communication.

        Args:
            config (dict): Configuration dictionary containing AGP settings. Must follow
                the structure defined in the AGP configuration reference:
                https://github.com/agntcy/agp/blob/main/data-plane/config/reference/config.yaml#L178-L289

            local_organization (str): Identifier for the organization running this server.
            local_namespace (str): Logical grouping identifier for resources in the local organization.
            local_agent (str): Identifier for this server instance.

        Note:
            This server should be used with a context manager (with statement) to ensure
            proper connection and disconnection of the gateway.
        """

        super().__init__(
            config,
            local_organization,
            local_namespace,
            local_agent,
        )
        self.active_sessions: Set[asyncio.Task] = set()
        self._lock = asyncio.Lock()

    async def _add_session(self, task: asyncio.Task) -> None:
        """Add a new session task to the active sessions."""
        async with self._lock:
            self.active_sessions.add(task)
            task.add_done_callback(self._remove_session)

    def _remove_session(self, task: asyncio.Task) -> None:
        """Remove a completed session task."""
        self.active_sessions.discard(task)

    async def cancel_all_sessions(self) -> None:
        """Cancel all active sessions."""
        async with self._lock:
            for task in self.active_sessions:
                if not task.done():
                    task.cancel()
            self.active_sessions.clear()

    async def handle_sessions(self, app: Server):
        """Handle incoming server sessions.

        Args:
            app: The MCP server application to handle sessions with.
        """
        try:
            async for session in self:
                assert session is not None, "Received null session"
                logger.info(f"New session received: {session}")

                async def run_session():
                    try:
                        async with self.new_streams(session) as streams:
                            await app.run(
                                streams[0], streams[1], app.create_initialization_options()
                            )
                    except Exception as e:
                        logger.error(f"Error in session {session}: {e}")
                        raise

                # Create and track the session task
                session_task = asyncio.create_task(run_session())
                await self._add_session(session_task)

        except Exception as e:
            logger.error(f"Error in handle_sessions: {e}")
            raise

    async def _send_message(
        self,
        session: agp_bindings.PySessionInfo,
        message: bytes,
    ):
        """
        Send a message to the next gateway.

        Args:
            session (agp_bindings.PySessionInfo): Session information.
            message (bytes): Message to send.

        Returns:
            MemoryObjectReceiveStream: Stream to receive the response.
        """

        if not self.gateway:
            raise RuntimeError(
                "Gateway is not connected. Please use the with statement."
            )

        # Send message to the gateway
        await self.gateway.publish_to(
            session,
            message,
        )

    def __aiter__(self):
        # make sure gateway is connected
        if not self.gateway:
            raise RuntimeError(
                "Gateway is not connected. Please use the with statement."
            )

        return self

    async def __anext__(self):
        # Wait for new session to be created
        session, _ = await self.gateway.receive()
        logger.debug(f"Received session: {session.id}")
        return session

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        """Clean up all sessions when exiting the context manager."""
        await self.cancel_all_sessions()
        await super().__aexit__(exc_type, exc_val, exc_tb)
