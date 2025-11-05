# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
import pytest

from google.protobuf.any_pb2 import Any as AnyMessage
from google.rpc import code_pb2, status_pb2

from slimrpc.context import Context
from slimrpc.rpc import RPCHandler, SRPCResponseError
from slimrpc.server import call_handler


async def _collect(generator):
    results = []
    async for item in generator:
        results.append(item)
    return results


@pytest.mark.asyncio
async def test_call_handler_unary_success() -> None:
    async def behaviour(request: str, context: Context) -> str:
        assert request == "ping"
        assert context.session_id == 123
        return "pong"

    handler = RPCHandler(
        behaviour=behaviour,
        request_deserializer=lambda x: x,
        response_serializer=lambda x: x,
        request_streaming=False,
        response_streaming=False,
    )

    context = Context(
        session_id=123,
        source_name="org/ns/client",
        destination_name="org/ns/server",
        payload_type="binary",
        metadata={"foo": "bar"},
    )

    results = await _collect(call_handler(handler, "ping", context))

    assert results == [(code_pb2.OK, "pong")]


@pytest.mark.asyncio
async def test_call_handler_streaming_responses() -> None:
    async def behaviour(request: str, context: Context):
        assert request == "stream"
        assert context.source_name == "org/ns/client"
        for item in ("one", "two", "three"):
            yield item

    handler = RPCHandler(
        behaviour=behaviour,
        request_deserializer=lambda x: x,
        response_serializer=lambda x: x,
        request_streaming=False,
        response_streaming=True,
    )

    context = Context(
        session_id=7,
        source_name="org/ns/client",
        destination_name="org/ns/server",
        payload_type="binary",
        metadata=None,
    )

    results = await _collect(call_handler(handler, "stream", context))

    assert results == [
        (code_pb2.OK, "one"),
        (code_pb2.OK, "two"),
        (code_pb2.OK, "three"),
    ]


@pytest.mark.asyncio
async def test_call_handler_translates_srpc_errors() -> None:
    detail = AnyMessage()
    detail.value = b"detail"

    async def behaviour(request: str, context: Context) -> str:
        raise SRPCResponseError(
            code_pb2.INVALID_ARGUMENT, "boom", details=[detail]
        )

    handler = RPCHandler(
        behaviour=behaviour,
        request_deserializer=lambda x: x,
        response_serializer=lambda x: x,
        request_streaming=False,
        response_streaming=False,
    )

    context = Context(
        session_id=99,
        source_name="org/ns/client",
        destination_name="org/ns/server",
        payload_type="binary",
        metadata={},
    )

    results = await _collect(call_handler(handler, "req", context))

    assert len(results) == 1
    code, payload = results[0]
    assert code == code_pb2.INVALID_ARGUMENT
    assert isinstance(payload, status_pb2.Status)
    assert payload.code == code_pb2.INVALID_ARGUMENT
    assert payload.message == "boom"
    assert list(payload.details) == [detail]


@pytest.mark.asyncio
async def test_call_handler_unknown_errors_become_internal() -> None:
    async def behaviour(request: str, context: Context) -> str:
        raise RuntimeError("unexpected failure")

    handler = RPCHandler(
        behaviour=behaviour,
        request_deserializer=lambda x: x,
        response_serializer=lambda x: x,
        request_streaming=False,
        response_streaming=False,
    )

    context = Context(
        session_id=5,
        source_name="org/ns/client",
        destination_name="org/ns/server",
        payload_type="binary",
        metadata=None,
    )

    results = await _collect(call_handler(handler, "req", context))

    assert len(results) == 1
    code, payload = results[0]
    assert code == code_pb2.UNKNOWN
    assert isinstance(payload, status_pb2.Status)
    assert payload.message == "Internal Error"
