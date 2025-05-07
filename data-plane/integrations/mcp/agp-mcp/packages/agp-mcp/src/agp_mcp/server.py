# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import logging

import agp_bindings

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

        Raises:
            RuntimeError: If the gateway is not connected.
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
        """
        Initialize the async iterator.

        Returns:
            AGPServer: The current instance of the AGPServer.

        Raises:
            RuntimeError: If the gateway is not connected.
        """

        # make sure gateway is connected
        if not self.gateway:
            raise RuntimeError(
                "Gateway is not connected. Please use the with statement."
            )

        return self

    async def __anext__(self):
        """Receive the next session from the gateway.

        This method is part of the async iterator protocol implementation. It waits for
        and receives the next session from the gateway.

        Returns:
            agp_bindings.PySessionInfo: The received session.
        """

        session, _ = await self.gateway.receive()
        logger.debug(f"Received session: {session.id}")

        return session
