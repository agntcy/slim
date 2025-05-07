# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
import random
import sys

import agp_bindings
import mcp.types as types
from readerwriterlock import rwlock

from agp_mcp.common import AGPBase

logger = logging.getLogger(__name__)

MAX_PENDING_PINGS = 3
PING_INTERVAL = 20


class AGPServer(AGPBase):
    # list of pending ping message per session
    # if the number of pendin pings exceed MAX_PENDING_PINGS
    # the client is not responsive anymore and the server
    # can close the session remove all the state
    pending_pings = {}
    # lock for the pending_pings
    rw_lock = rwlock.RWLockFair()

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

    def clean_session(self, session: agp_bindings.PySessionInfo):
        with self.rw_lock.gen_wlock():
            self.pending_pings[session.id] = []

    def remove_session(self, session: agp_bindings.PySessionInfo):
        with self.rw_lock.gen_wlock():
            del self.pending_pings[session.id]

    def add_ping_id(self, session: agp_bindings.PySessionInfo, id: int) -> bool:
        with self.rw_lock.gen_wlock():
            if session.id in self.pending_pings:
                v = self.pending_pings[session.id]
                v.append(id)
                if len(v) > MAX_PENDING_PINGS:
                    logger.debug(
                        f"Maximum number of pending pings reached in session {session.id}"
                    )
                    return True
                self.pending_pings[session.id] = v
            else:
                self.pending_pings[session.id] = [id]

        return False

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

    def _filter_message(
        self,
        session: agp_bindings.PySessionInfo,
        message: types.JSONRPCMessage,
    ) -> bool:
        clean = False
        if isinstance(message.root, types.JSONRPCResponse):
            response: type.JSONRPCResponse = message.root
            if response.result == {}:
                with self.rw_lock.gen_rlock():
                    if session.id in self.pending_pings:
                        v = self.pending_pings[session.id]
                        if response.id in v:
                            # got a reply so clean the state and return True
                            logger.debug(f"Received ping reply on session {session.id}")
                            clean = True
        if clean:
            self.clean_session(session)
            return True

        return False

    async def _ping(self, session: agp_bindings.PySessionInfo):
        while True:
            id = random.randint(0, sys.maxsize)
            if self.add_ping_id(session, id):
                return

            message = types.JSONRPCMessage(
                root=types.JSONRPCRequest(jsonrpc="2.0", id=id, method="ping")
            )
            json = message.model_dump_json(by_alias=True, exclude_none=True)
            await self._send_message(session, json.encode())
            await asyncio.sleep(PING_INTERVAL)

    def _stop_session(
        self,
        session: agp_bindings.PySessionInfo,
    ) -> bool:
        remove_session = False
        with self.rw_lock.gen_rlock():
            if session.id in self.pending_pings:
                v = self.pending_pings[session.id]
                if len(v) > MAX_PENDING_PINGS:
                    remove_session = True

        if remove_session:
            self.remove_session(session)
            return True

        return False

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
