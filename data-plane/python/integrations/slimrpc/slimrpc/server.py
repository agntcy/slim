# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
import sys
import time
from collections.abc import AsyncGenerator, AsyncIterable, Callable
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
    RequestType,
    SLIMAppConfig,
    create_local_app,
    handler_name_to_pyname,
)
from slimrpc.context import Context
from slimrpc.rpc import RPCHandler, SRPCResponseError

logger = logging.getLogger(__name__)


@dataclass(unsafe_hash=True)
class ServiceMethod:
    service: str
    method: str


class Server:
    def __init__(
        self,
        slim_app_config: SLIMAppConfig | None = None,
        local_app: slim_bindings.Slim | None = None,
    ) -> None:
        if slim_app_config is None and local_app is None:
            raise ValueError("Either slim_app_config or local_app must be provided")
        if slim_app_config is not None and local_app is not None:
            raise ValueError("Only one of slim_app_config or local_app can be provided")
        self._slim_app_config = slim_app_config
        self._local_app = local_app
        self._local_app_lock = asyncio.Lock()
        self.handlers: dict[ServiceMethod, RPCHandler] = {}
        self._pyname_to_handler: dict[slim_bindings.PyName, RPCHandler] = {}

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

    async def get_local_app(self) -> slim_bindings.Slim:
        async with self._local_app_lock:
            if self._local_app is None:
                if self._slim_app_config is None:
                    raise ValueError(
                        "Either slim_app_config or local_app must be provided"
                    )
                self._local_app = await create_local_app(self._slim_app_config)
                await self._local_app.__aenter__()
            return self._local_app

    async def run(self) -> None:
        """
        Run the server, creating a local SLIM instance and subscribing to the handlers.
        """

        # Create local SLIM instance
        local_app = await self.get_local_app()

        logger.info(f"Subscribing to {local_app.local_name.id}")

        await local_app.subscribe(local_app.local_name)
        logger.info(
            f"Subscribing to {local_app.local_name}",
        )

        # Subscribe and store mapping to subscription name for session handling
        # later.
        self._pyname_to_handler = {}
        for service_method, handler in self.handlers.items():
            subscription_name = handler_name_to_pyname(
                local_app.local_name, service_method.service, service_method.method
            )
            strs = subscription_name.components_strings()
            s_clone = slim_bindings.PyName(
                strs[0], strs[1], strs[2], local_app.local_name.id
            )
            logger.info(
                f"Subscribing to {s_clone}",
            )
            await local_app.subscribe(s_clone)
            self._pyname_to_handler[s_clone] = handler

        instance = local_app.get_id()

        try:
            # Wait for a message and reply in a loop
            while True:
                logger.info(f"{instance} waiting for new session to be established")

                session_info, _ = await local_app.receive()
                logger.info(
                    f"{instance} received a new session: {session_info.id}",
                )

                asyncio.create_task(self.handle_session(session_info, local_app))
        finally:
            await self.close()

    async def close(self) -> None:
        async with self._local_app_lock:
            if self._local_app is not None:
                if self._slim_app_config is None:
                    logger.debug("not closing local app as it was provided externally")
                    return
                await self._local_app.__aexit__(None, None, None)
            self._local_app = None

    async def handle_session(
        self, session_info: slim_bindings.PySessionInfo, local_app: slim_bindings.Slim
    ) -> None:
        logger.info(f"new session from {session_info.source_name}")

        # Find the RPC handler for the session
        rpc_handler: RPCHandler | None = self._pyname_to_handler.get(
            session_info.destination_name, None
        )
        if rpc_handler is None:
            logger.error(
                f"no handler found for session {session_info.id} with destination {session_info.destination_name}",
            )
            return

        # Call the RPC handler
        try:
            # Get deadline from request
            deadline_str = session_info.metadata.get(DEADLINE_KEY, "")
            timeout = (
                max(float(deadline_str) - time.time(), 0)
                if deadline_str
                else MAX_TIMEOUT
            )

            async with asyncio_timeout(timeout):
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
            logger.warn(f"session {session_info.id} timed out after {timeout} seconds")
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
) -> AsyncGenerator[Tuple[Any, Any], None]:
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
