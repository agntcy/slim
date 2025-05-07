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

# Configuration constants
CONFIG_ENDPOINT_KEY = "endpoint"


class AGPBase(ABC):
    """Base class for AGP (Agent Gateway Protocol) communication.

    This abstract base class provides the core functionality for AGP communication,
    including connection management, session handling, and message routing.

    Attributes:
        config (Dict[str, Any]): Configuration dictionary containing connection settings
        local_organization (str): Local organization identifier
        local_namespace (str): Local namespace identifier
        local_agent (str): Local agent identifier
        remote_organization (Optional[str]): Remote organization identifier
        remote_namespace (Optional[str]): Remote namespace identifier
        remote_mcp_agent (Optional[str]): Remote MCP agent identifier
    """

    def __init__(
        self,
        config: dict[str, Any],
        local_organization: str,
        local_namespace: str,
        local_agent: str,
        remote_organization: str | None = None,
        remote_namespace: str | None = None,
        remote_mcp_agent: str | None = None,
    ):
        """Initialize the AGP base class.

        Args:
            config: Configuration dictionary containing connection settings
            local_organization: Local organization identifier
            local_namespace: Local namespace identifier
            local_agent: Local agent identifier
            remote_organization: Remote organization identifier
            remote_namespace: Remote namespace identifier
            remote_mcp_agent: Remote MCP agent identifier

        Raises:
            ValueError: If required configuration is missing
        """
        if CONFIG_ENDPOINT_KEY not in config:
            raise ValueError(
                f"Missing required configuration key: {CONFIG_ENDPOINT_KEY}"
            )

        self.config = config
        self.local_organization = local_organization
        self.local_namespace = local_namespace
        self.local_agent = local_agent

        self.remote_organization = remote_organization
        self.remote_namespace = remote_namespace
        self.remote_mcp_agent = remote_mcp_agent

        self.gateway: agp_bindings.Gateway

    def is_connected(self) -> bool:
        """Check if the client is connected to the gateway.

        Returns:
            bool: True if connected, False otherwise
        """
        return self.gateway is not None

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
        """Initialize and connect to the AGP gateway.

        Returns:
            self: The initialized instance

        Raises:
            RuntimeError: If gateway initialization fails
            ValueError: If configuration is invalid
            ConnectionError: If connection to the gateway fails
        """
        try:
            # Initialize the AGP gateway
            self.gateway = await agp_bindings.Gateway.new(
                self.local_organization, self.local_namespace, self.local_agent
            )

            # connect to AGP server
            logger.info(
                f"Connecting to AGP server: {self.config}",
            )
            try:
                await self.gateway.connect(self.config)
            except Exception as e:
                logger.error(
                    "Failed to connect to AGP server",
                    extra={
                        "error": str(e),
                        "endpoint": self.config[CONFIG_ENDPOINT_KEY],
                    },
                    exc_info=True,
                )
                raise ConnectionError(
                    f"Failed to connect to AGP server: {str(e)}"
                ) from e

            # Set route if remote details are provided
            if all(
                [self.remote_organization, self.remote_namespace, self.remote_mcp_agent]
            ):
                try:
                    await self.gateway.set_route(
                        self.remote_organization,
                        self.remote_namespace,
                        self.remote_mcp_agent,
                    )
                    logger.info(
                        "Route set successfully",
                        extra={
                            "remote_org": self.remote_organization,
                            "remote_namespace": self.remote_namespace,
                            "remote_agent": self.remote_mcp_agent,
                        },
                    )
                except Exception as e:
                    logger.error(
                        "Failed to set route",
                        extra={
                            "error": str(e),
                            "remote_org": self.remote_organization,
                            "remote_namespace": self.remote_namespace,
                            "remote_agent": self.remote_mcp_agent,
                        },
                        exc_info=True,
                    )
                    raise RuntimeError(f"Failed to set route: {str(e)}") from e

            # start receiving messages
            try:
                await self.gateway.__aenter__()
            except Exception as e:
                logger.error("Failed to start receiving messages", exc_info=True)
                raise RuntimeError(
                    f"Failed to start receiving messages: {str(e)}"
                ) from e

            return self

        except Exception as e:
            logger.error("Failed to initialize AGP gateway", exc_info=True)
            raise RuntimeError(f"Failed to initialize AGP gateway: {str(e)}") from e

    async def __aexit__(self, exc_type: type[Any], exc_value: Any, traceback: Any):
        # Disconnect from the AGP server
        if self.gateway:
            await self.gateway.__aexit__(exc_type, exc_value, traceback)
            self.gateway = None

    @asynccontextmanager
    async def new_streams(
        self,
        accepted_session: agp_bindings.PySessionInfo,
    ):
        """Create a new session for message exchange.

        Args:
            accepted_session: Optional session info to use instead of the default session

        Yields:
            Tuple[MemoryObjectReceiveStream, MemoryObjectSendStream]: Streams for reading and writing messages

        Raises:
            ValueError: If no valid session is available
        """
        # initialize streams
        read_stream: MemoryObjectReceiveStream[types.JSONRPCMessage | Exception]
        read_stream_writer: MemoryObjectSendStream[types.JSONRPCMessage | Exception]

        write_stream: MemoryObjectSendStream[types.JSONRPCMessage]
        write_stream_reader: MemoryObjectReceiveStream[types.JSONRPCMessage]

        read_stream_writer, read_stream = anyio.create_memory_object_stream(0)
        write_stream, write_stream_reader = anyio.create_memory_object_stream(0)

        async def agp_reader():
            session = accepted_session
            try:
                while True:
                    try:
                        session, msg = await self.gateway.receive(session=session.id)
                        logger.debug(
                            "Received message", extra={"message": msg.decode()}
                        )

                        message = types.JSONRPCMessage.model_validate_json(msg.decode())
                        await read_stream_writer.send(message)
                    except Exception as exc:
                        logger.error("Error receiving message", exc_info=True)
                        await read_stream_writer.send(exc)
                        break
            finally:
                await read_stream_writer.aclose()

        async def agp_writer():
            try:
                async for message in write_stream_reader:
                    try:
                        json = message.model_dump_json(by_alias=True, exclude_none=True)
                        logger.debug("Sending message", extra={"message": json})
                        await self._send_message(accepted_session, json.encode())
                    except Exception:
                        logger.error("Error sending message", exc_info=True)
                        raise
            finally:
                await write_stream_reader.aclose()

        async with anyio.create_task_group() as tg:
            tg.start_soon(agp_reader)
            tg.start_soon(agp_writer)
            try:
                yield read_stream, write_stream
            finally:
                # cancel the task group
                tg.cancel_scope.cancel()
                # delete the session
                await self.gateway.delete_session(accepted_session.id)
