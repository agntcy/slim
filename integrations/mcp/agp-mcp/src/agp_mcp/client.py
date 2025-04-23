# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import logging

import agp_bindings

from agp_mcp.common import AGPBase

logger = logging.getLogger(__name__)


class AGPClient(AGPBase):
    def __init__(
        self,
        config: dict,
        local_organization: str,
        local_namespace: str,
        local_agent: str,
        remote_organization: str,
        remote_namespace: str,
        remote_mcp_agent: str,
    ):
        """
        Server transport for AGP.

        Args:
            config (dict): Configuration dictionary. This config should reflect the
                configuration struct defined in AGP. A reference can be found in
                https://github.com/agntcy/agp/blob/main/data-plane/config/reference/config.yaml#L58-L172

            local_organization (str): Local organization name.
            local_namespace (str): Local namespace name.
            local_agent (str): Local agent name.
            remote_organization (str | None): Remote organization name.
            remote_namespace (str | None): Remote namespace name.
            remote_mcp_agent (str | None): Remote MCP agent name.
        """

        super().__init__(
            config,
            local_organization,
            local_namespace,
            local_agent,
            remote_organization,
            remote_namespace,
            remote_mcp_agent,
        )

    async def _send_message(
        self,
        session: agp_bindings.PySessionInfo,
        message: bytes,
    ):
        """
        Send a message to the next gateway

        Args:
            session (agp_bindings.PySessionInfo): Session information.
            message (bytes): Message to send.
        """

        if not self.gateway:
            raise RuntimeError(
                "Gateway is not connected. Please use the with statement."
            )

        # Send message to the gateway
        await self.gateway.publish(
            session,
            message,
            self.remote_organization,
            self.remote_namespace,
            self.remote_mcp_agent,
        )
