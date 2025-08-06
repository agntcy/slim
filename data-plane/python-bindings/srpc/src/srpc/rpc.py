# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import logging
from collections.abc import AsyncIterable, Awaitable
from typing import Any

from google.rpc import code_pb2, status_pb2

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
        method_name: str,
        handler: Awaitable,  # Or Callable?
        request_deserializer: callable = lambda x: x,
        response_serializer: callable = lambda x: x,
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

    async def call_handler(self, *args, **kwargs) -> tuple[int, Any]:
        """
        Call the handler with the given arguments.
        """

        code = 0

        try:
            response = await self.handler(*args, **kwargs)
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

        return code, response


def unary_unary_rpc_method_handler(
    method_name: str,
    handler: Awaitable,
    request_deserializer: callable = lambda x: x,
    response_serializer: callable = lambda x: x,
) -> Rpc:
    return Rpc(
        method_name="ExampleUnaryUnary",
        handler=handler,
        request_deserializer=request_deserializer,
        response_serializer=response_serializer,
        request_streaming=False,
        response_streaming=False,
    )


def unary_stream_rpc_method_handler(
    method_name: str,
    handler: AsyncIterable,
    request_deserializer: callable = lambda x: x,
    response_serializer: callable = lambda x: x,
) -> Rpc:
    return Rpc(
        method_name=method_name,
        handler=handler,
        request_deserializer=request_deserializer,
        response_serializer=response_serializer,
        request_streaming=False,
        response_streaming=True,
    )


def stream_unary_rpc_method_handler(
    method_name: str,
    handler: Awaitable,
    request_deserializer: callable = lambda x: x,
    response_serializer: callable = lambda x: x,
) -> Rpc:
    return Rpc(
        method_name=method_name,
        handler=handler,
        request_deserializer=request_deserializer,
        response_serializer=response_serializer,
        request_streaming=True,
        response_streaming=False,
    )


def stream_stream_rpc_method_handler(
    method_name: str,
    handler: Awaitable,
    request_deserializer: callable = lambda x: x,
    response_serializer: callable = lambda x: x,
) -> Rpc:
    return Rpc(
        method_name=method_name,
        handler=handler,
        request_deserializer=request_deserializer,
        response_serializer=response_serializer,
        request_streaming=True,
        response_streaming=True,
    )
