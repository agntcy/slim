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
        slim_client_configs: list[dict[str, Any]],
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
            slim_client_configs,
            local_organization,
            local_namespace,
            local_agent,
            remote_organization,
            remote_namespace,
            remote_mcp_agent,
            message_timeout=message_timeout,
            message_retries=message_retries,
        )

    @asynccontextmanager
    async def to_mcp_session(self, *args, **kwargs):
        """Create a new MCP session.

        Returns:
            slim_bindings.Session: The new MCP session
        """
        # create session
        session, ack = await self.slim.create_session(
            destination=self.remote_svc_name,
            session_config=slim_bindings.SessionConfiguration.PointToPoint(
                max_retries=self.message_retries,
                timeout=self.message_timeout,
            ),
        )
        await ack

        # create streams
        try:
            async with self.new_streams(session) as (read_stream, write_stream):
                async with ClientSession(
                    read_stream, write_stream, *args, **kwargs
                ) as mcp_session:
                    yield mcp_session
        finally:
            ack = await self.slim.delete_session(session)
            await ack
