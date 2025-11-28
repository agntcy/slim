# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
from abc import ABC
from contextlib import asynccontextmanager
import datetime
from typing import Any

import slim_bindings
import anyio
import mcp.types as types
from anyio.streams.memory import MemoryObjectReceiveStream, MemoryObjectSendStream

from slim_mcp.helpers import create_local_app

logger = logging.getLogger(__name__)


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
        slim_client_configs: list[dict[str, Any]],
        local_organization: str,
        local_namespace: str,
        local_agent: str,
        remote_organization: str | None = None,
        remote_namespace: str | None = None,
        remote_mcp_agent: str | None = None,
        shared_secret: str = "secretsecretsecretsecretsecretsecret",
        enable_opentelemetry: bool = False,
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
        self.slim_client_configs = slim_client_configs
        self.local = slim_bindings.Name(
            local_organization, local_namespace, local_agent
        )
        self.shared_secret = shared_secret
        self.enable_opentelemetry = enable_opentelemetry

        self.remote_svc_name: slim_bindings.Name | None = None
        if remote_organization and remote_namespace and remote_mcp_agent:
            self.remote_svc_name = slim_bindings.Name(
                remote_organization,
                remote_namespace,
                remote_mcp_agent,
            )

        self.slim: slim_bindings.Slim | None

        self.message_timeout = message_timeout
        self.message_retries = message_retries

    def is_connected(self) -> bool:
        """Check if the client is connected to slim.

        Returns:
            bool: True if connected, False otherwise
        """
        return self.slim is not None

    def _filter_message(
        self,
        session: slim_bindings.Session,
        message: types.JSONRPCMessage,
        pendin_pings: list[int],
    ) -> bool:
        """
        Check the message content. If it returns True the message should be
        dropped and not pass to the application

        Args:
            session (slim_bindings.Session): SLIM session.
            message (types.JSONRPCMessage): Message to control.

        Returns:
            bool: True if the message has to be dropped, False otherwise
        """

        return False

    async def _ping(
        self,
        session: slim_bindings.Session,
        pendin_pings: list[int],
    ):
        """
        Send an MCP ping message to the other endpoint

        Args:
            session (slim_bindings.Session): SLIM session.
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
            self.slim = await create_local_app(
                self.local,
                self.slim_client_configs,
                enable_opentelemetry=self.enable_opentelemetry,
                shared_secret=self.shared_secret,
            )

            # Set route if remote details are provided
            if self.remote_svc_name is not None and len(self.slim_client_configs) > 0:
                try:
                    await self.slim.set_route(
                        self.remote_svc_name,
                    )
                    logger.info(
                        "Route set successfully",
                        extra={
                            "remote_svc": str(self.remote_svc_name),
                        },
                    )
                except Exception as e:
                    logger.error(
                        "Failed to set route",
                        extra={
                            "error": str(e),
                            "remote_svc": str(self.remote_svc_name),
                        },
                        exc_info=True,
                    )
                    raise RuntimeError(f"Failed to set route: {str(e)}") from e

            return self

        except Exception as e:
            logger.error("Failed to initialize SLIM instance", exc_info=True)
            raise RuntimeError(f"Failed to initialize SLIM instance: {str(e)}") from e

    async def __aexit__(self, exc_type: type[Any], exc_value: Any, traceback: Any):
        # Disconnect from the SLIM server
        if self.slim:
            self.slim = None

    @asynccontextmanager
    async def new_streams(
        self,
        accepted_session: slim_bindings.Session,
    ):
        """Create a new session for message exchange.

        Args:
            accepted_session: Session to use for message exchange

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

        class TerminateTaskGroup(Exception):
            pass

        async def slim_reader(session: slim_bindings.Session):
            try:
                while True:
                    try:
                        msg_ctx, msg = await session.get_message()
                        logger.debug(
                            "Received message", extra={"message": msg.decode()}
                        )

                        message = types.JSONRPCMessage.model_validate_json(msg.decode())
                        if not self._filter_message(
                            accepted_session, message, pending_pings
                        ):
                            await read_stream_writer.send(message)
                    except Exception as exc:
                        # The client closes the session when it wants to
                        # terminate the session, raise TerminateTaskGroup to
                        # cancel the task group and all the streams.
                        if "session channel closed" in str(exc):
                            raise TerminateTaskGroup()

                        logger.error("Error receiving message", exc_info=True)
                        await read_stream_writer.send(exc)
                        break
            finally:
                await read_stream_writer.aclose()

        async def slim_writer(session: slim_bindings.Session):
            session = accepted_session
            try:
                async for message in write_stream_reader:
                    try:
                        json = message.model_dump_json(by_alias=True, exclude_none=True)
                        logger.debug("Sending message", extra={"mcp_message": json})
                        ack = await session.publish(json.encode())
                        await ack
                    except Exception:
                        logger.error("Error sending message", exc_info=True)
                        raise
            finally:
                await write_stream_reader.aclose()

        async def ping(group, session: slim_bindings.Session):
            try:
                t1 = asyncio.create_task(self._ping(session, pending_pings))
                await t1
            finally:
                if len(pending_pings) != 0:
                    raise TerminateTaskGroup()
                else:
                    t1.cancel()

        async def force_termination():
            raise TerminateTaskGroup()

        try:
            async with asyncio.TaskGroup() as tg:
                tg.create_task(slim_reader(accepted_session))
                tg.create_task(slim_writer(accepted_session))
                tg.create_task(ping(tg, accepted_session))

                yield read_stream, write_stream

                # cancel the task group when the context manager hands back
                # control indicating the consumer is done.
                tg.create_task(force_termination())
        except* TerminateTaskGroup:
            pass
