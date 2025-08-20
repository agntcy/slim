# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
import sys
from collections.abc import AsyncGenerator, AsyncIterable, Callable
from typing import Any, Tuple

if sys.version_info >= (3, 11):
    from asyncio import timeout_at as asyncio_timeout_at
else:
    from async_timeout import timeout_at as asyncio_timeout_at

import slim_bindings
from google.rpc import code_pb2, status_pb2

from srpc.common import (
    DEADLINE_KEY,
    MAX_TIMEOUT,
    RequestType,
    create_local_app,
    handler_name_to_pyname,
    split_id,
)
from srpc.context import Context
from srpc.rpc import RPCHandler, SRPCResponseError

logger = logging.getLogger(__name__)


class Server:
    def __init__(
        self,
        local: str,
        slim: dict,
        enable_opentelemetry: bool = False,
        shared_secret: str = "",
    ) -> None:
        self.local_name = split_id(local)
        self.slim = slim
        self.enable_opentelemetry = enable_opentelemetry
        self.shared_secret = shared_secret
        self.handlers: dict[slim_bindings.PyName, RPCHandler] = {}

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

        # Compose a PyName using the fist components of the local name and the RPC name
        subscription_name = handler_name_to_pyname(
            self.local_name, service_name, method_name
        )

        # Register the RPC handler
        self.handlers[subscription_name] = rpc_handler

    async def run(self) -> None:
        """
        Run the server, creating a local SLIM instance and subscribing to the handlers.
        """

        # Create local SLIM instance
        local_app = await create_local_app(
            self.local_name,
            self.slim,
            enable_opentelemetry=self.enable_opentelemetry,
            shared_secret=self.shared_secret,
        )

        logger.info(f"Subscribing to {local_app.local_name.id}")

        await local_app.subscribe(local_app.local_name)
        logger.info(
            f"Subscribing to {local_app.local_name}",
        )

        # Subscribe
        new_handlers = {}
        for s, h in self.handlers.items():
            strs = s.components_strings()
            s_clone = slim_bindings.PyName(
                strs[0], strs[1], strs[2], local_app.local_name.id
            )
            logger.info(
                f"Subscribing to {s_clone}",
            )
            await local_app.subscribe(s_clone)

            new_handlers[s_clone] = h

        self.handlers = new_handlers

        instance = local_app.get_id()

        async with local_app:
            # Wait for a message and reply in a loop
            while True:
                logger.info(f"{instance} waiting for new session to be established")

                session_info, _ = await local_app.receive()
                logger.info(
                    f"{instance} received a new session: {session_info.id}",
                )

                asyncio.create_task(self.handle_session(session_info, local_app))

    async def handle_session(
        self, session_info: slim_bindings.PySessionInfo, local_app: slim_bindings.Slim
    ) -> None:
        rpc_handler: RPCHandler = self.handlers[session_info.destination_name]

        logger.info(f"new session from {session_info.source_name}")

        # Call the RPC handler
        if session_info.destination_name not in self.handlers:
            logger.error(
                f"no handler found for session {session_info.id} with destination {session_info.destination_name}",
            )
            return

        try:
            # Get deadline from request
            deadline = float(session_info.metadata.get(DEADLINE_KEY, str(MAX_TIMEOUT)))

            async with asyncio_timeout_at(deadline):
                if not rpc_handler.request_streaming:
                    # Read the request from slim
                    session_recv, request_bytes = await local_app.receive(
                        session=session_info.id,
                    )

                    request_or_iterator = rpc_handler.request_deserializer(
                        request_bytes
                    )
                    context = Context.from_sessioninfo(session_recv)
                else:
                    request_or_iterator, context = (
                        request_generator(
                            local_app, rpc_handler.request_deserializer, session_info
                        ),
                        Context.from_sessioninfo(session_info),
                    )

                # Send the response back to the client
                async for code, response in call_handler(
                    rpc_handler, request_or_iterator, context
                ):
                    await local_app.publish_to(
                        session_info,
                        rpc_handler.response_serializer(response),
                        metadata={"code": str(code)},
                    )

                # Send end of stream message if the response was streaming
                if rpc_handler.response_streaming:
                    await local_app.publish_to(
                        session_info,
                        b"",
                        metadata={"code": str(code_pb2.OK)},
                    )
        except asyncio.TimeoutError:
            pass
        finally:
            logger.info(f"deleting session {session_info.id}")
            await local_app.delete_session(session_info.id)


def request_generator(
    local_app: slim_bindings.Slim,
    request_deserializer: Callable,
    session_info: slim_bindings.PySessionInfo,
) -> AsyncGenerator[Any, Context]:
    async def generator() -> AsyncGenerator[Any, Context]:
        try:
            while True:
                session_recv, request_bytes = await local_app.receive(
                    session=session_info.id,
                )

                if (
                    session_recv.metadata.get("code") == str(code_pb2.OK)
                    and not request_bytes
                ):
                    logger.debug("end of stream received")
                    break

                request = request_deserializer(request_bytes)
                context = Context.from_sessioninfo(session_recv)

                yield request, context
        except Exception as e:
            logger.error(f"Error receiving messages from SLIM: {e}")
            raise

    return generator()


async def call_handler(
    handler: RPCHandler,
    request_or_iterator: RequestType | AsyncIterable[RequestType],
    context: Context,
) -> AsyncGenerator[Tuple[Any, Any]]:
    """
    Call the handler with the given arguments.
    """
    try:
        if not handler.response_streaming:
            # If not streaming we expect a single response and then exit the generator.
            response = await handler.behaviour(request_or_iterator, context)
            yield code_pb2.OK, response
            return

        # For streaming responses yield each response from the handler
        async for response in handler.behaviour(request_or_iterator, context):
            yield code_pb2.OK, response
    except SRPCResponseError as e:
        logger.error("Error while calling handler 1")
        yield (
            e.code,
            status_pb2.Status(code=e.code, message=e.message, details=e.details),
        )
    except Exception as e:
        logger.error(f"Error while calling handler 2 {e}")
        yield (
            code_pb2.UNKNOWN,
            status_pb2.Status(
                code=code_pb2.UNKNOWN, message="Internal Error", details=None
            ),
        )
