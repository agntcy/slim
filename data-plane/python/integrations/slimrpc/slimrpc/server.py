# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import logging
import sys
import time
from asyncio import TimeoutError, create_task
from collections.abc import AsyncGenerator, Callable
from dataclasses import dataclass
from typing import Any, Tuple

if sys.version_info >= (3, 11):
    from asyncio import timeout as asyncio_timeout
else:
    from async_timeout import timeout as asyncio_timeout

import slim_bindings
from google.rpc import code_pb2, status_pb2

from slimrpc.common import (
    DEADLINE_KEY,
    MAX_TIMEOUT,
    SLIMAppConfig,
    create_local_app,
    handler_name_to_pyname,
)
from slimrpc.context import MessageContext, SessionContext
from slimrpc.rpc import RPCHandler, SRPCResponseError

logger = logging.getLogger(__name__)


@dataclass(unsafe_hash=True)
class ServiceMethod:
    service: str
    method: str


class Server:
    def __init__(
        self,
        local_app: slim_bindings.Slim,
    ) -> None:
        self._local_app = local_app
        self.handlers: dict[ServiceMethod, RPCHandler] = {}
        self._pyname_to_handler: dict[slim_bindings.Name, RPCHandler] = {}

    @classmethod
    async def from_slim_app_config(cls, slim_app_config: SLIMAppConfig) -> "Server":
        local_app = await create_local_app(slim_app_config)
        return cls(local_app=local_app)

    def register_method_handlers(
        self, service_name: str, handlers: dict[str, RPCHandler]
    ) -> None:
        """
        Register method handlers for the server.
        """
        for method_name, handler in handlers.items():
            self.register_rpc(service_name, method_name, handler)

    def register_rpc(
        self, service_name: str, method_name: str, rpc_handler: RPCHandler
    ) -> None:
        """
        Register an RPC handler for the server.
        """

        # Register the RPC handler
        self.handlers[ServiceMethod(service=service_name, method=method_name)] = (
            rpc_handler
        )

    async def run(self) -> None:
        """
        Run the server, creating a local SLIM instance and subscribing to the handlers.
        """

        logger.info(f"Subscribing to {self._local_app.local_name.id}")

        await self._local_app.subscribe(self._local_app.local_name)
        logger.info(
            f"Subscribing to {self._local_app.local_name}",
        )

        # Subscribe and store mapping to subscription name for session handling
        # later.
        self._pyname_to_handler = {}
        for service_method, handler in self.handlers.items():
            subscription_name = handler_name_to_pyname(
                self._local_app.local_name,
                service_method.service,
                service_method.method,
            )
            strs = subscription_name.components_strings()
            s_clone = slim_bindings.Name(
                strs[0], strs[1], strs[2], self._local_app.local_name.id
            )
            logger.info(
                f"Subscribing to {s_clone}",
            )
            await self._local_app.subscribe(s_clone)
            self._pyname_to_handler[s_clone] = handler

        instance = self._local_app.id_str

        # Wait for a message and reply in a loop
        while True:
            logger.info(f"{instance} waiting for new session to be established")

            session = await self._local_app.listen_for_session()
            logger.info(
                f"{instance} received a new session: {session.id}",
            )

            create_task(self.handle_session(session))

    async def handle_session(
        self,
        session: slim_bindings.Session,
    ) -> None:
        logger.info(f"new session from {session.dst} to {session.src}")

        # Call the RPC handler
        try:
            # Find the RPC handler for the session
            rpc_handler: RPCHandler | None = self._pyname_to_handler.get(
                session.src, None
            )
            if rpc_handler is None:
                logger.error(
                    f"no handler found for session {session.id} with destination {session.src}",
                )
                return

            # Get deadline from request
            deadline_str = session.metadata.get(DEADLINE_KEY, "")
            timeout = (
                max(float(deadline_str) - time.time(), 0)
                if deadline_str
                else MAX_TIMEOUT
            )

            async with asyncio_timeout(timeout):
                # Send the response back to the client
                async for code, response in call_handler(
                    rpc_handler,
                    session,
                ):
                    logger.info(
                        f"sending response for session {session.id} with code {code}"
                    )
                    ack = await session.publish(
                        rpc_handler.response_serializer(response),
                        metadata={"code": str(code)},
                    )
                    await ack

                # Send end of stream message if the response was streaming
                if rpc_handler.response_streaming:
                    ack = await session.publish(
                        b"",
                        metadata={"code": str(code_pb2.OK)},
                    )
                    await ack
        except Exception as e:
            if isinstance(e, TimeoutError):
                logger.warn(f"session {session.id} timed out after {timeout} seconds")
            else:
                logger.error(f"error handling session {session.id}: {e}")

            # Only close the session if there is an uncaught exception, most
            # errors should be caught and bubbled up to the client in the RPC
            # communication, but if there is another issue or we timeout we
            # close our own session because it'll let the client know something
            # went wrong.
            await self.try_close_session(session)

    async def try_close_session(self, session: slim_bindings.Session) -> None:
        try:
            await self._local_app.delete_session(session)
        except Exception as e:
            logger.error(f"Error closing session: {e}")


async def request_generator(
    session: slim_bindings.Session,
    request_deserializer: Callable,
) -> AsyncGenerator[Any, MessageContext]:
    try:
        while True:
            msg_ctx, request_bytes = await session.get_message()

            if msg_ctx.metadata.get("code") == str(code_pb2.OK) and not request_bytes:
                logger.debug("end of stream received")
                break

            request = request_deserializer(request_bytes)
            context = MessageContext.from_message_context(msg_ctx)
            yield request, context
    except Exception as e:
        logger.error(f"Error receiving messages from SLIM: {e}")
        raise


async def call_handler(
    handler: RPCHandler,
    session: slim_bindings.Session,
) -> AsyncGenerator[Tuple[Any, Any], None]:
    """
    Call the handler with the given arguments.
    """
    try:
        session_context = SessionContext.from_session(session)

        handler_args: Any
        if not handler.request_streaming:
            # For unary requests, get the single message
            msg_ctx, request_bytes = await session.get_message()
            request = handler.request_deserializer(request_bytes)
            handler_args = (
                request,
                MessageContext.from_message_context(msg_ctx),
                session_context,
            )
        else:
            # For streaming requests, create a generator
            request_iterator = request_generator(session, handler.request_deserializer)
            handler_args = (request_iterator, session_context)

        if not handler.response_streaming:
            # If not streaming we expect a single response and then exit the generator.
            response = await handler.behaviour(*handler_args)
            yield code_pb2.OK, response
            return

        # For streaming responses yield each response from the handler
        async for response in handler.behaviour(*handler_args):
            yield code_pb2.OK, response
    except SRPCResponseError as e:
        yield (
            e.code,
            status_pb2.Status(code=e.code, message=e.message, details=e.details),
        )
    except Exception:
        logger.exception("Unexpected error while calling handler")
        yield (
            code_pb2.UNKNOWN,
            status_pb2.Status(
                code=code_pb2.UNKNOWN, message="Internal Error", details=None
            ),
        )
