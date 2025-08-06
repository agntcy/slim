# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
from contextlib import asynccontextmanager

from google.rpc import code_pb2, status_pb2

import anyio

import slim_bindings
from srpc.common import (
    create_local_app,
    handler_name_to_pyname,
    split_id,
)
from srpc.context import Context
from srpc.rpc import ErrorResponse, Rpc

logging.basicConfig(level=logging.DEBUG)
logger = logging.getLogger(__name__)


class Server:
    def __init__(
        self,
        local: str,
        slim: dict,
        enable_opentelemetry: bool = False,
        shared_secret: str | None = None,
    ):
        self.local_name = split_id(local)
        self.slim = slim
        self.enable_opentelemetry = enable_opentelemetry
        self.shared_secret = shared_secret

        self.handlers = {}

        self.local_app: slim_bindings.Slim = None

    def register_method_handlers(self, service_name: str, handlers: dict[str, Rpc]):
        """
        Register method handlers for the server.
        """
        for method_name, handler in handlers.items():
            handler.method_name = method_name
            handler.service_name = service_name

            self.register_rpc(handler)

    def register_rpc(self, rpc_handler: Rpc):
        """
        Register an RPC handler for the server.
        """

        # Compose a PyName using the fist components of the local name and the RPC name
        subscription_name = handler_name_to_pyname(self.local_name, rpc_handler)

        # Register the RPC handler
        self.handlers[subscription_name] = rpc_handler

    async def run(self):
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

        # Subscribe
        for s, h in self.handlers.items():
            logger.info(
                f"Subscribing to {s}",
            )
            await local_app.subscribe(s)

        instance = local_app.get_id()

        async with local_app:
            # Wait for a message and reply in a loop
            while True:
                logging.info(f"{instance} waiting for new session to be established")

                session_info, _ = await local_app.receive()
                logging.info(
                    f"{instance} received a new session: {session_info.id}",
                )

                asyncio.create_task(self.handle_session(session_info, local_app))

    async def handle_session(
        self, session_info: slim_bindings.PySessionInfo, local_app: slim_bindings.Slim
    ):
        instance = local_app.get_id()

        rpc_handler: Rpc = self.handlers[session_info.destination_name]

        async with self.multi_callable_streams(
            local_app,
            session_info,
            rpc_handler.request_deserializer,
            rpc_handler.response_serializer,
        ) as (read_stream, write_stream):
            # Call the RPC handler
            if session_info.destination_name not in self.handlers:
                logger.error(
                    f"{instance} no handler found for session {session_info.id} with destination {session_info.destination_name}",
                )
                return

            if not rpc_handler.request_streaming:
                # Read the request from the stream
                request, context = await read_stream.receive()

                if isinstance(request, Exception):
                    logging.error(
                        f"{instance} error reading request: {request}",
                        exc_info=True,
                    )
                    return
            else:
                request, context = read_stream, Context.from_sessioninfo(session_info)

            logger.info(f"Handling request {request} with context {context}")
            if not rpc_handler.response_streaming:
                logger.info(f"handling unary response")

                # Call the handler with the request and context
                code, response = await rpc_handler.call_handler(request, context)

                # Create generator to send the response
                async def generator():
                    yield response

                response_generator = generator()
            else:
                # If the response is streaming, we need to handle it differently
                logger.info(f"handling streaming response")
                response_generator = rpc_handler.handler(request, context)

            # Send the response back to the client
            code = 0
            try:
                async for response in response_generator:
                    await write_stream.send((response, {"code": code}))
            except ErrorResponse as e:
                logger.error("Error while calling handler 1")
                response = status_pb2.Status(
                    code=e.code, message=e.message, details=e.details
                )
                code = e.code
            except Exception as e:
                logger.error(f"Error while calling handler 2 {e}")
                response = status_pb2.Status(
                    code=code_pb2.UNKNOWN, message="Internal Error", details=None
                )
                code = code_pb2.UNKNOWN

    @asynccontextmanager
    async def multi_callable_streams(
        self,
        local_app: slim_bindings.Slim,
        session: slim_bindings.PySessionInfo,
        request_deserializer: callable = lambda x: x,
        response_serializer: callable = lambda x: x,
    ):
        # initialize streams
        read_stream: anyio.MemoryObjectReceiveStream
        read_stream_writer: anyio.MemoryObjectSendStream

        write_stream: anyio.MemoryObjectSendStream
        write_stream_reader: anyio.MemoryObjectReceiveStream

        read_stream_writer, read_stream = anyio.create_memory_object_stream(0)
        write_stream, write_stream_reader = anyio.create_memory_object_stream(0)

        async def slim_reader():
            try:
                while True:
                    try:
                        recv_session, request_bytes = await local_app.receive(
                            session=session.id
                        )
                        logger.debug("Received message")

                        request = request_deserializer(request_bytes)
                        context = Context.from_sessioninfo(recv_session)

                        await read_stream_writer.send((request, context))
                    except Exception as exc:
                        logger.error("Error receiving message", exc_info=True)
                        await read_stream_writer.send(exc)
                        break
            finally:
                await read_stream_writer.aclose()

        async def slim_writer():
            try:
                async for response, code in write_stream_reader:
                    try:
                        response_bytes = response_serializer(response)
                        logger.debug("Sending message")
                        await local_app.publish_to(
                            session,
                            response_bytes,
                            # metadata={"code": code}
                        )
                    except Exception:
                        logger.error("Error sending message", exc_info=True)
                        raise
            finally:
                await write_stream_reader.aclose()

        async with anyio.create_task_group() as tg:
            tg.start_soon(slim_reader)
            tg.start_soon(slim_writer)
            try:
                yield read_stream, write_stream
            finally:
                # cancel the task group
                tg.cancel_scope.cancel()
