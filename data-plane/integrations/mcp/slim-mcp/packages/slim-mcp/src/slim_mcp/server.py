# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime
import logging
import random
import sys

import slim_bindings
import mcp.types as types

from slim_mcp.common import SLIMBase

logger = logging.getLogger(__name__)

MAX_PENDING_PINGS = 3
PING_INTERVAL = 20


class SLIMServer(SLIMBase):
    def __init__(
        self,
        config: dict,
        local_organization: str,
        local_namespace: str,
        local_agent: str,
        message_timeout: datetime.timedelta = datetime.timedelta(seconds=15),
        message_retries: int = 2,
    ):
        """
        SLIM transport Server for MCP (Model Context Protocol) communication.

        Args:
            config (dict): Configuration dictionary containing SLIM settings. Must follow
                the structure defined in the SLIM configuration reference:
                https://github.com/agntcy/slim/blob/main/data-plane/config/reference/config.yaml#L178-L289

            local_organization (str): Identifier for the organization running this server.
            local_namespace (str): Logical grouping identifier for resources in the local organization.
            local_agent (str): Identifier for this server instance.

        Note:
            This server should be used with a context manager (with statement) to ensure
            proper connection and disconnection of SLIM.
        """

        super().__init__(
            config,
            local_organization,
            local_namespace,
            local_agent,
        )

    async def _send_message(
        self,
        session: slim_bindings.PySessionInfo,
        message: bytes,
    ):
        """
        Send a message to the next slim instance.

        Args:
            session (slim_bindings.PySessionInfo): Session information.
            message (bytes): Message to send.

        Raises:
            RuntimeError: If SLIM is not connected.
        """

        if not self.slim:
            raise RuntimeError("SLIM is not connected. Please use the with statement.")

        # Send message to slim
        await self.slim.publish_to(
            session,
            message,
        )

    def _filter_message(
        self,
        session: slim_bindings.PySessionInfo,
        message: types.JSONRPCMessage,
        pending_pings: list[int],
    ) -> bool:
        if isinstance(message.root, types.JSONRPCResponse):
            response: types.JSONRPCResponse = message.root
            if response.result == {}:
                if response.id in pending_pings:
                    logger.debug(f"Received ping reply on session {session.id}")
                    pending_pings.clear()
                    return True

        return False

    async def _ping(
        self, session: slim_bindings.PySessionInfo, pending_pings: list[int]
    ):
        while True:
            id = random.randint(0, sys.maxsize)
            pending_pings.append(id)

            if len(pending_pings) > MAX_PENDING_PINGS:
                logger.debug(
                    f"Maximum number of pending pings reached in session {session.id}"
                )
                return

            message = types.JSONRPCMessage(
                root=types.JSONRPCRequest(jsonrpc="2.0", id=id, method="ping")
            )
            json = message.model_dump_json(by_alias=True, exclude_none=True)
            await self._send_message(session, json.encode())
            await asyncio.sleep(PING_INTERVAL)

    def __aiter__(self):
        """
        Initialize the async iterator.

        Returns:
            SLIMServer: The current instance of the SLIMServer.

        Raises:
            RuntimeError: If slim is not connected.
        """

        # make sure slim is connected
        if not self.slim:
            raise RuntimeError("SLIM is not connected. Please use the with statement.")

        return self

    async def __anext__(self):
        """Receive the next session from SLIM.

        This method is part of the async iterator protocol implementation. It waits for
        and receives the next session from the SLIM.

        Returns:
            slim_bindings.PySessionInfo: The received session.
        """

        session, _ = await self.slim.receive()
        logger.debug(f"Received session: {session.id}")

        return session
