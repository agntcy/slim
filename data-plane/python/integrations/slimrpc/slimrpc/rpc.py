# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import logging
from collections.abc import Callable
from dataclasses import dataclass
from typing import Iterable, Mapping, Optional, Union

from google.protobuf.any_pb2 import Any as pb_Any
from google.rpc import code_pb2

logger = logging.getLogger(__name__)


class SRPCResponseError(Exception):
    def __init__(
        self,
        code: code_pb2.Code,
        message: str,
        details: Optional[Iterable[Union[pb_Any, Mapping]]] = None,
    ) -> None:
        self.code = code
        self.message = message
        self.details = details
        super().__init__(message)


@dataclass
class RPCHandler:
    behaviour: Callable
    request_deserializer: Callable
    response_serializer: Callable
    request_streaming: bool = False
    response_streaming: bool = False


def unary_unary_rpc_method_handler(
    behaviour: Callable,
    request_deserializer: Callable = lambda x: x,
    response_serializer: Callable = lambda x: x,
) -> RPCHandler:
    return RPCHandler(
        behaviour=behaviour,
        request_deserializer=request_deserializer,
        response_serializer=response_serializer,
        request_streaming=False,
        response_streaming=False,
    )


def unary_stream_rpc_method_handler(
    behaviour: Callable,
    request_deserializer: Callable = lambda x: x,
    response_serializer: Callable = lambda x: x,
) -> RPCHandler:
    return RPCHandler(
        behaviour=behaviour,
        request_deserializer=request_deserializer,
        response_serializer=response_serializer,
        request_streaming=False,
        response_streaming=True,
    )


def stream_unary_rpc_method_handler(
    behaviour: Callable,
    request_deserializer: Callable = lambda x: x,
    response_serializer: Callable = lambda x: x,
) -> RPCHandler:
    return RPCHandler(
        behaviour=behaviour,
        request_deserializer=request_deserializer,
        response_serializer=response_serializer,
        request_streaming=True,
        response_streaming=False,
    )


def stream_stream_rpc_method_handler(
    behaviour: Callable,
    request_deserializer: Callable = lambda x: x,
    response_serializer: Callable = lambda x: x,
) -> RPCHandler:
    return RPCHandler(
        behaviour=behaviour,
        request_deserializer=request_deserializer,
        response_serializer=response_serializer,
        request_streaming=True,
        response_streaming=True,
    )
