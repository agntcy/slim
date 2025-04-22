# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import logging
from contextlib import asynccontextmanager
from typing import Any
from urllib.parse import urljoin, urlparse

import anyio
from anyio.abc import TaskStatus
from anyio.streams.memory import MemoryObjectReceiveStream, MemoryObjectSendStream
import agp_bindings

import mcp.types as types

logger = logging.getLogger(__name__)


class AGPClient:
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
        """

        self.config = config
        self.local_organization = local_organization
        self.local_namespace = local_namespace
        self.local_agent = local_agent

        self.remote_organization = remote_organization
        self.remote_namespace = remote_namespace
        self.remote_mcp_agent = remote_mcp_agent

        self.gateway = None

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
        logger.debug(
            f"Setting route to {self.remote_organization}/{self.remote_namespace}/{self.remote_mcp_agent}"
        )
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
    ):
        # initialize streams
        read_stream: MemoryObjectReceiveStream[types.JSONRPCMessage | Exception]
        read_stream_writer: MemoryObjectSendStream[types.JSONRPCMessage | Exception]

        write_stream: MemoryObjectSendStream[types.JSONRPCMessage]
        write_stream_reader: MemoryObjectReceiveStream[types.JSONRPCMessage]

        read_stream_writer, read_stream = anyio.create_memory_object_stream(0)
        write_stream, write_stream_reader = anyio.create_memory_object_stream(0)

        async def agp_reader():
            session_local = self.session

            while True:
                try:
                    session_local, msg = await self.gateway.receive(
                        session=session_local.id
                    )
                    logger.debug(f"Received message: {msg}")

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
                await self.gateway.publish(
                    self.session,
                    json.encode(),
                    self.remote_organization,
                    self.remote_namespace,
                    self.remote_mcp_agent,
                )

        async with anyio.create_task_group() as tg:
            tg.start_soon(agp_reader)
            tg.start_soon(agp_writer)
            try:
                yield read_stream, write_stream
            finally:
                tg.cancel_scope.cancel()
