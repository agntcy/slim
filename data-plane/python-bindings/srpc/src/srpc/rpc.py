# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import logging
from collections.abc import AsyncGenerator, AsyncIterable, Awaitable
from typing import Any

from google.rpc import code_pb2, status_pb2

import slim_bindings
from srpc.context import Context

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


class ErrorResponse(Exception):
    def __init__(self, code, message, details=None):
        self.code = code
        self.message = message
        self.details = details
        super().__init__(message)


class Rpc:
    """
    Base class for RPC object. It holds the method name, handler, serializers, and
    deserializers for the request and response.
    """

    def __init__(
        self,
        handler: Awaitable,  # Or Callable?
        request_deserializer: callable = lambda x: x,
        response_serializer: callable = lambda x: x,
        method_name: str | None = None,
        service_name: str | None = None,
        request_streaming: bool = False,
        response_streaming: bool = False,
    ):
        self.service_name = service_name
        self.method_name = method_name
        self.handler = handler
        self.request_deserializer = request_deserializer
        self.response_serializer = response_serializer
        self.request_streaming = request_streaming
        self.response_streaming = response_streaming

    def request_generator(
        self, local_app: slim_bindings.Slim, session_info: slim_bindings.PySessionInfo
    ) -> AsyncGenerator[Any, Context]:
        async def generator():
            try:
                while True:
                    session_recv, request_bytes = await local_app.receive(
                        session=session_info.id,
                    )

                    if not self.request_streaming:
                        break

                    if (
                        session_recv.metadata.get("code") == str(code_pb2.OK)
                        and not request_bytes
                    ):
                        logger.info("End of stream received")
                        break

                    request = self.request_deserializer(request_bytes)
                    context = Context.from_sessioninfo(session_recv)

                    yield request, context
            except Exception as e:
                logger.error(f"Error receiving messages from SLIM: {e}")
                raise

        return generator()

    async def call_handler(self, *args, **kwargs) -> AsyncGenerator[int, Any]:
        """
        Call the handler with the given arguments.
        """

        code = code_pb2.OK

        try:
            if not self.response_streaming:
                response = await self.handler(*args, **kwargs)
                yield code, response
            else:
                async for response in self.handler(*args, **kwargs):
                    yield code, response

            return
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

        yield code, response


def unary_unary_rpc_method_handler(
    handler: Awaitable,
    request_deserializer: callable = lambda x: x,
    response_serializer: callable = lambda x: x,
) -> Rpc:
    return Rpc(
        handler=handler,
        request_deserializer=request_deserializer,
        response_serializer=response_serializer,
        request_streaming=False,
        response_streaming=False,
    )


def unary_stream_rpc_method_handler(
    handler: AsyncIterable,
    request_deserializer: callable = lambda x: x,
    response_serializer: callable = lambda x: x,
) -> Rpc:
    return Rpc(
        handler=handler,
        request_deserializer=request_deserializer,
        response_serializer=response_serializer,
        request_streaming=False,
        response_streaming=True,
    )


def stream_unary_rpc_method_handler(
    handler: Awaitable,
    request_deserializer: callable = lambda x: x,
    response_serializer: callable = lambda x: x,
) -> Rpc:
    return Rpc(
        handler=handler,
        request_deserializer=request_deserializer,
        response_serializer=response_serializer,
        request_streaming=True,
        response_streaming=False,
    )


def stream_stream_rpc_method_handler(
    handler: Awaitable,
    request_deserializer: callable = lambda x: x,
    response_serializer: callable = lambda x: x,
) -> Rpc:
    return Rpc(
        handler=handler,
        request_deserializer=request_deserializer,
        response_serializer=response_serializer,
        request_streaming=True,
        response_streaming=True,
    )
