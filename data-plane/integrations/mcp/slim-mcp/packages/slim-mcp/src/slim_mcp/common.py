# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
from abc import ABC, abstractmethod
from contextlib import asynccontextmanager
import datetime
from typing import Any

import slim_bindings
import anyio
import mcp.types as types
from anyio.streams.memory import MemoryObjectReceiveStream, MemoryObjectSendStream

logger = logging.getLogger(__name__)

# Configuration constants
CONFIG_ENDPOINT_KEY = "endpoint"


class SLIMBase(ABC):
    """Base class for SLIM communication.

    This abstract base class provides the core functionality for SLIM communication,
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
        message_timeout: datetime.timedelta = datetime.timedelta(seconds=15),
        message_retries: int = 2,
    ):
        """Initialize the SLIM base class.

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

        self.slim: slim_bindings.Slim

        self.message_timeout = message_timeout
        self.message_retries = message_retries

    def is_connected(self) -> bool:
        """Check if the client is connected to slim.

        Returns:
            bool: True if connected, False otherwise
        """
        return self.slim is not None

    @abstractmethod
    async def _send_message(
        self,
        session: slim_bindings.PySessionInfo,
        message: bytes,
    ):
        """
        Send a message to the SLIM server.

        Args:
            session (slim_bindings.PySessionInfo): SLIM session info.
            message (bytes): Message to send.
        """

        # This method should be implemented in subclasses.
        pass

    def _filter_message(
        self,
        session: slim_bindings.PySessionInfo,
        message: types.JSONRPCMessage,
        pendin_pings: list[int],
    ) -> bool:
        """
        Check the message content. If it returns True the message should be
        dropped and not pass to the application

        Args:
            session (slim_bindings.PySessionInfo): SLIM session info.
            message (types.JSONRPCMessage): Message to control.

        Returns:
            bool: True if the message has to be dropped, False otherwise
        """

        return False

    async def _ping(
        self,
        session: slim_bindings.PySessionInfo,
        pendin_pings: list[int],
    ):
        """
        Send an MCP ping message to the other endpoint

        Args:
            session (slim_bindings.PySessionInfo): SLIM session info.
        """

        pass

    async def __aenter__(self):
        """Initialize and connect to the SLIM instance.

        Returns:
            self: The initialized instance

        Raises:
            RuntimeError: If slim initialization fails
            ValueError: If configuration is invalid
            ConnectionError: If connection to slim fails
        """
        try:
            # Initialize the SLIM instance
            self.slim = await slim_bindings.Slim.new(
                self.local_organization, self.local_namespace, self.local_agent
            )

            # connect to SLIM server
            logger.info(
                f"Connecting to SLIM server: {self.config}",
            )
            try:
                await self.slim.connect(self.config)
            except Exception as e:
                logger.error(
                    "Failed to connect to SLIM server",
                    extra={
                        "error": str(e),
                        "endpoint": self.config[CONFIG_ENDPOINT_KEY],
                    },
                    exc_info=True,
                )
                raise ConnectionError(
                    f"Failed to connect to SLIM server: {str(e)}"
                ) from e

            # Set route if remote details are provided
            if all(
                [self.remote_organization, self.remote_namespace, self.remote_mcp_agent]
            ):
                try:
                    await self.slim.set_route(
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

            # Set default fire and forget session configuration to be reliable
            try:
                await self.slim.set_default_session_config(
                    slim_bindings.PySessionConfiguration.FireAndForget(
                        timeout=self.message_timeout,
                        max_retries=self.message_retries,
                    )
                )
            except Exception as e:
                logger.error(
                    "Failed to set default session configuration",
                    extra={
                        "error": str(e),
                        "remote_org": self.remote_organization,
                        "remote_namespace": self.remote_namespace,
                        "remote_agent": self.remote_mcp_agent,
                    },
                    exc_info=True,
                )
                raise RuntimeError(
                    f"Failed to set default session configuration: {str(e)}"
                ) from e

            # start receiving messages
            try:
                await self.slim.__aenter__()
            except Exception as e:
                logger.error("Failed to start receiving messages", exc_info=True)
                raise RuntimeError(
                    f"Failed to start receiving messages: {str(e)}"
                ) from e

            return self

        except Exception as e:
            logger.error("Failed to initialize SLIM instance", exc_info=True)
            raise RuntimeError(f"Failed to initialize SLIM instance: {str(e)}") from e

    async def __aexit__(self, exc_type: type[Any], exc_value: Any, traceback: Any):
        # Disconnect from the SLIM server
        if self.slim:
            await self.slim.__aexit__(exc_type, exc_value, traceback)
            self.slim = None

    @asynccontextmanager
    async def new_streams(
        self,
        accepted_session: slim_bindings.PySessionInfo,
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

        pending_pings: list = []

        async def slim_reader():
            session = accepted_session
            try:
                while True:
                    try:
                        session, msg = await self.slim.receive(session=session.id)
                        logger.debug(
                            "Received message", extra={"message": msg.decode()}
                        )

                        message = types.JSONRPCMessage.model_validate_json(msg.decode())
                        if not self._filter_message(
                            accepted_session, message, pending_pings
                        ):
                            await read_stream_writer.send(message)
                    except Exception as exc:
                        logger.error("Error receiving message", exc_info=True)
                        await read_stream_writer.send(exc)
                        break
            finally:
                await read_stream_writer.aclose()

        async def slim_writer():
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

        async def ping():
            session = accepted_session
            try:
                t1 = asyncio.create_task(self._ping(session, pending_pings))
                await t1
            finally:
                if len(pending_pings) != 0:
                    tg.cancel_scope.cancel()
                else:
                    t1.cancel()

        async with anyio.create_task_group() as tg:
            tg.start_soon(slim_reader)
            tg.start_soon(slim_writer)
            tg.start_soon(ping)
            try:
                yield read_stream, write_stream
            finally:
                # cancel the task group
                tg.cancel_scope.cancel()
                # delete the session
                logger.info(
                    f"Closing session: {accepted_session.id}",
                )
                await self.slim.delete_session(accepted_session.id)
