# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import logging
from abc import ABC, abstractmethod
from contextlib import asynccontextmanager
from typing import Any

import agp_bindings
import anyio
import mcp.types as types
from anyio.streams.memory import MemoryObjectReceiveStream, MemoryObjectSendStream

logger = logging.getLogger(__name__)


class AGPBase(ABC):
    def __init__(
        self,
        config: dict,
        local_organization: str,
        local_namespace: str,
        local_agent: str,
        remote_organization: str | None = None,
        remote_namespace: str | None = None,
        remote_mcp_agent: str | None = None,
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

        self.config = config
        self.local_organization = local_organization
        self.local_namespace = local_namespace
        self.local_agent = local_agent

        self.remote_organization = remote_organization
        self.remote_namespace = remote_namespace
        self.remote_mcp_agent = remote_mcp_agent

        self.gateway = None

    @abstractmethod
    async def _send_message(
        self,
        session: agp_bindings.PySessionInfo,
        message: bytes,
    ):
        """
        Send a message to the AGP server.

        Args:
            message (bytes): Message to send.
        """

        # This method should be implemented in subclasses.
        pass

    async def __aenter__(self):
        # Initialize the AGP gateway
        self.gateway = await agp_bindings.Gateway.new(
            self.local_organization, self.local_namespace, self.local_agent
        )

        # connect to AGP server
        logger.debug(f"Connecting to AGP server: {self.config['endpoint']}")
        await self.gateway.connect(
            self.config,
        )

        # Set route
        if self.remote_organization and self.remote_namespace and self.remote_mcp_agent:
            await self.gateway.set_route(
                self.remote_organization,
                self.remote_namespace,
                self.remote_mcp_agent,
            )

            logger.debug(
                f"Route set to {self.remote_organization}/{self.remote_namespace}/{self.remote_mcp_agent}"
            )

        # start receiving messages
        await self.gateway.__aenter__()

        # create session
        self.session = await self.gateway.create_session(
            agp_bindings.PySessionConfiguration.FireAndForget()
        )

        return self

    async def __aexit__(self, exc_type: type[Any], exc_value: Any, traceback: Any):
        # Disconnect from the AGP server
        if self.gateway:
            await self.gateway.__aexit__(exc_type, exc_value, traceback)
            self.gateway = None

    @asynccontextmanager
    async def new_session(
        self,
        accepted_session: agp_bindings.PySessionInfo | None = None,
    ):
        # initialize streams
        read_stream: MemoryObjectReceiveStream[types.JSONRPCMessage | Exception]
        read_stream_writer: MemoryObjectSendStream[types.JSONRPCMessage | Exception]

        write_stream: MemoryObjectSendStream[types.JSONRPCMessage]
        write_stream_reader: MemoryObjectReceiveStream[types.JSONRPCMessage]

        read_stream_writer, read_stream = anyio.create_memory_object_stream(0)
        write_stream, write_stream_reader = anyio.create_memory_object_stream(0)

        if not accepted_session:
            accepted_session = self.session

        # if both sessions are None, raise an error
        if accepted_session is None:
            raise ValueError(
                "Either accepted_session or self.session must be provided."
            )

        async def agp_reader():
            session = accepted_session

            while True:
                try:
                    session, msg = await self.gateway.receive(session=session.id)
                    logger.debug(f"Received message: {msg}")
                    print(f"Received message: {msg}")

                    message = types.JSONRPCMessage.model_validate_json(msg.decode())
                except Exception as exc:
                    logging.error(f"Error receiving message: {exc}")
                    await read_stream_writer.send(exc)
                    continue

                # send message to read stream
                await read_stream_writer.send(message)

        async def agp_writer():
            async for message in write_stream_reader:
                json = message.model_dump_json(by_alias=True, exclude_none=True)
                print(f"Send message: {json}")
                await self._send_message(
                    accepted_session,
                    json.encode(),
                )
                logger.debug(f"Sent message: {json}")

        async with anyio.create_task_group() as tg:
            tg.start_soon(agp_reader)
            tg.start_soon(agp_writer)
            try:
                yield read_stream, write_stream
            finally:
                tg.cancel_scope.cancel()
