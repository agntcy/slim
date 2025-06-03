# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import logging
from contextlib import asynccontextmanager
import datetime
from typing import Any

import slim_bindings
from mcp import ClientSession

from slim_mcp.common import SLIMBase

logger = logging.getLogger(__name__)


class SLIMClient(SLIMBase):
    """
    SLIM transport client for MCP (Model Context Protocol) communication.

    Attributes:
        config (Dict[str, Any]): Configuration dictionary containing connection settings
        local_organization (str): Local organization identifier
        local_namespace (str): Local namespace identifier
        local_agent (str): Local agent identifier
        remote_organization (str): Remote organization identifier
        remote_namespace (str): Remote namespace identifier
        remote_mcp_agent (str): Remote MCP agent identifier
    """

    def __init__(
        self,
        config: dict[str, Any],
        local_organization: str,
        local_namespace: str,
        local_agent: str,
        remote_organization: str,
        remote_namespace: str,
        remote_mcp_agent: str,
        message_timeout: datetime.timedelta = datetime.timedelta(seconds=15),
        message_retries: int = 2,
    ) -> None:
        """
        Initialize the SLIM client.

        Args:
            config: Configuration dictionary containing SLIM connection settings. Must follow
                the structure defined in the SLIM configuration reference:
                https://github.com/agntcy/slim/blob/main/data-plane/config/reference/config.yaml#L58-L172
            local_organization: Local organization identifier
            local_namespace: Local namespace identifier
            local_agent: Local agent identifier
            remote_organization: Remote organization identifier
            remote_namespace: Remote namespace identifier
            remote_mcp_agent: Remote MCP agent identifier

        Raises:
            ValueError: If any of the required parameters are empty or invalid
        """

        super().__init__(
            config,
            local_organization,
            local_namespace,
            local_agent,
            remote_organization,
            remote_namespace,
            remote_mcp_agent,
            message_timeout=message_timeout,
            message_retries=message_retries,
        )

    async def _send_message(
        self,
        session: slim_bindings.PySessionInfo,
        message: bytes,
    ) -> None:
        """Send a message to the remote slim instance.

        Args:
            session: Session information for the message
            message: Message to send in bytes format

        Raises:
            RuntimeError: If SLIM is not connected or if sending fails
        """
        if not self.is_connected():
            raise RuntimeError("SLIM is not connected. Please use the with statement.")

        try:
            logger.debug(
                "Sending message to remote slim instance",
                extra={
                    "remote_org": self.remote_organization,
                    "remote_namespace": self.remote_namespace,
                    "remote_agent": self.remote_mcp_agent,
                },
            )
            # Send message to SLIM instance
            await self.slim.publish(
                session,
                message,
                self.remote_organization,
                self.remote_namespace,
                self.remote_mcp_agent,
            )
            logger.debug("Message sent successfully")
        except Exception as e:
            logger.error("Failed to send message", exc_info=True)
            raise RuntimeError(f"Failed to send message: {str(e)}") from e

    @asynccontextmanager
    async def to_mcp_session(self, *args, **kwargs):
        """Create a new MCP session.

        Returns:
            slim_bindings.PySessionInfo: The new MCP session
        """
        # create session
        session = await self.slim.create_session(
            slim_bindings.PySessionConfiguration.FireAndForget(
                timeout=self.message_timeout,
                max_retries=self.message_retries,
                sticky=True,
            )
        )

        # create streams
        async with self.new_streams(session) as (read_stream, write_stream):
            async with ClientSession(
                read_stream, write_stream, *args, **kwargs
            ) as mcp_session:
                yield mcp_session
