# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import contextlib
from collections.abc import AsyncIterable, AsyncIterator
from unittest.mock import AsyncMock, MagicMock

import pytest
import slim_bindings
from google.rpc import code_pb2

from slimrpc import channel as channel_module
from slimrpc import common as common_module
from slimrpc import server as server_module
from slimrpc.context import Context
from slimrpc.rpc import (
    stream_unary_rpc_method_handler,
    unary_stream_rpc_method_handler,
    unary_unary_rpc_method_handler,
)


@pytest.fixture
def mock_session_info() -> MagicMock:
    """Create a mock session info object."""
    session_info = MagicMock(spec=slim_bindings.PySessionInfo)
    session_info.id = 1
    session_info.source_name = slim_bindings.PyName("org", "ns", "client")
    session_info.destination_name = slim_bindings.PyName("org", "ns", "server")
    session_info.metadata = {common_module.DEADLINE_KEY: "5.0"}
    return session_info


@pytest.fixture
def mock_server_app() -> AsyncMock:
    """Create a mock server app."""
    app = AsyncMock()
    app.local_name = slim_bindings.PyName("org", "ns", "server")
    app.get_id.return_value = "org/ns/server"
    app.subscribe = AsyncMock()
    app.publish_to = AsyncMock()
    app.delete_session = AsyncMock()
    return app


@pytest.fixture
def mock_client_app() -> AsyncMock:
    """Create a mock client app."""
    app = AsyncMock()
    app.local_name = slim_bindings.PyName("org", "ns", "client")
    app.get_id.return_value = "org/ns/client"
    app.set_route = AsyncMock()
    app.create_session = AsyncMock()
    app.publish = AsyncMock()
    app.receive = AsyncMock()
    app.delete_session = AsyncMock()

    # Support async context manager
    app.__aenter__ = AsyncMock(return_value=app)
    app.__aexit__ = AsyncMock()

    return app


@pytest.mark.asyncio
async def test_unary_unary_handler_execution(
    mock_server_app: AsyncMock, mock_session_info: MagicMock
) -> None:
    """Test that unary-unary RPC handler is called correctly and response is sent."""
    server = server_module.Server(local_app=mock_server_app)

    # Create a test handler
    handler_called = False
    received_request = None
    received_context = None

    async def behaviour(request: bytes, context: Context) -> bytes:
        nonlocal handler_called, received_request, received_context
        handler_called = True
        received_request = request
        received_context = context
        return request.upper()

    handler = unary_unary_rpc_method_handler(
        behaviour,
        request_deserializer=lambda payload: payload,
        response_serializer=lambda payload: payload,
    )

    server.register_rpc("TestService", "Echo", handler)

    # Set up handler mapping to simulate what _prepare_server does
    # Map the destination_name directly to the handler
    server._pyname_to_handler[mock_session_info.destination_name] = handler

    # Mock receiving the request
    request_payload = b"hello"
    mock_server_app.receive.side_effect = [
        (mock_session_info, request_payload),
        asyncio.CancelledError(),  # End the session
    ]

    # Handle the session
    with contextlib.suppress(asyncio.CancelledError):
        await server.handle_session(mock_session_info, mock_server_app)

    # Verify handler was called with correct data
    assert handler_called
    assert received_request == request_payload
    assert received_context is not None
    assert received_context.metadata == mock_session_info.metadata

    # Verify response was sent back
    mock_server_app.publish_to.assert_called_once()
    call_args = mock_server_app.publish_to.call_args
    assert call_args[0][1] == b"HELLO"  # Response payload


@pytest.mark.asyncio
async def test_unary_stream_handler_execution(
    mock_server_app: AsyncMock, mock_session_info: MagicMock
) -> None:
    """Test that unary-stream RPC handler streams multiple responses."""
    server = server_module.Server(local_app=mock_server_app)

    async def behaviour(request: bytes, context: Context) -> AsyncIterator[bytes]:
        assert request == b"hello"
        for i in range(3):
            yield request + f"-{i}".encode()

    handler = unary_stream_rpc_method_handler(
        behaviour,
        request_deserializer=lambda payload: payload,
        response_serializer=lambda payload: payload,
    )

    server.register_rpc("TestService", "Stream", handler)

    # Set up handler mapping
    server._pyname_to_handler[mock_session_info.destination_name] = handler

    # Mock receiving the request
    mock_server_app.receive.side_effect = [
        (mock_session_info, b"hello"),
        asyncio.CancelledError(),
    ]

    # Handle the session
    with contextlib.suppress(asyncio.CancelledError):
        await server.handle_session(mock_session_info, mock_server_app)

    # Verify multiple responses were sent (3 data + 1 end-of-stream)
    assert mock_server_app.publish_to.call_count == 4

    # Check each response (data payloads)
    calls = mock_server_app.publish_to.call_args_list
    assert calls[0][0][1] == b"hello-0"
    assert calls[1][0][1] == b"hello-1"
    assert calls[2][0][1] == b"hello-2"
    # Fourth call is end-of-stream marker
    assert calls[3][0][1] == b""


@pytest.mark.asyncio
async def test_stream_unary_handler_execution(
    mock_server_app: AsyncMock, mock_session_info: MagicMock
) -> None:
    """Test that stream-unary RPC handler aggregates multiple requests."""
    server = server_module.Server(local_app=mock_server_app)

    received_chunks = []

    async def behaviour(
        request_iterator: AsyncIterable[tuple[bytes, Context]], context: Context
    ) -> bytes:
        async for chunk, _chunk_context in request_iterator:
            received_chunks.append(chunk)
        return b"|".join(received_chunks)

    handler = stream_unary_rpc_method_handler(
        behaviour,
        request_deserializer=lambda payload: payload,
        response_serializer=lambda payload: payload,
    )

    server.register_rpc("TestService", "Aggregate", handler)

    # Set up handler mapping
    server._pyname_to_handler[mock_session_info.destination_name] = handler

    # Create a mock for the "code" metadata check
    chunk_info1 = MagicMock()
    chunk_info1.metadata = {"code": str(code_pb2.OK)}
    chunk_info2 = MagicMock()
    chunk_info2.metadata = {"code": str(code_pb2.OK)}
    chunk_info3 = MagicMock()
    chunk_info3.metadata = {"code": str(code_pb2.OK)}
    end_of_stream_info = MagicMock()
    end_of_stream_info.metadata = {"code": str(code_pb2.OK)}

    # Mock receiving multiple request chunks + end-of-stream marker
    mock_server_app.receive.side_effect = [
        (chunk_info1, b"first"),
        (chunk_info2, b"second"),
        (chunk_info3, b"third"),
        (end_of_stream_info, b""),  # End of stream marker
    ]

    # Handle the session
    await server.handle_session(mock_session_info, mock_server_app)

    # Verify all chunks were received
    assert received_chunks == [b"first", b"second", b"third"]

    # Verify aggregated response was sent
    mock_server_app.publish_to.assert_called_once()
    call_args = mock_server_app.publish_to.call_args
    assert call_args[0][1] == b"first|second|third"


@pytest.mark.asyncio
async def test_channel_unary_unary_call(mock_client_app: AsyncMock) -> None:
    """Test that channel correctly sends unary request and receives response."""
    channel_factory = channel_module.ChannelFactory(local_app=mock_client_app)
    channel = channel_factory.new_channel("org/ns/server")

    # Create mock session
    mock_session_info = MagicMock()
    mock_session_info.id = 1
    mock_session_info.metadata = {}
    mock_client_app.create_session.return_value = mock_session_info

    # Mock receiving the response with proper code
    response_session_info = MagicMock()
    response_session_info.metadata = {"code": str(code_pb2.OK)}
    mock_client_app.receive.return_value = (response_session_info, b"RESPONSE")

    # Make the call
    call = channel.unary_unary(
        "/TestService/Echo",
        request_serializer=lambda x: x,
        response_deserializer=lambda x: x,
    )

    response = await call(b"request", timeout=5)

    # Verify session was created
    mock_client_app.create_session.assert_called_once()

    # Verify request was published
    mock_client_app.publish.assert_called_once()
    publish_call = mock_client_app.publish.call_args
    assert publish_call[0][1] == b"request"  # Payload
    assert (
        common_module.DEADLINE_KEY in publish_call[1]["metadata"]
    )  # Metadata has deadline

    # Verify response was received
    assert response == b"RESPONSE"

    # Verify session was deleted
    mock_client_app.delete_session.assert_called_once_with(mock_session_info.id)


@pytest.mark.asyncio
async def test_channel_unary_stream_call(mock_client_app: AsyncMock) -> None:
    """Test that channel correctly receives streaming responses."""
    channel_factory = channel_module.ChannelFactory(local_app=mock_client_app)
    channel = channel_factory.new_channel("org/ns/server")

    # Create mock session
    mock_session_info = MagicMock()
    mock_session_info.id = 1
    mock_session_info.metadata = {}
    mock_client_app.create_session.return_value = mock_session_info

    # Mock receiving multiple responses with proper code
    response_metadata = {"code": str(code_pb2.OK)}
    mock_client_app.receive.side_effect = [
        (MagicMock(metadata=response_metadata), b"response-1"),
        (MagicMock(metadata=response_metadata), b"response-2"),
        (MagicMock(metadata=response_metadata), b"response-3"),
        asyncio.CancelledError(),  # End stream
    ]

    # Make the call
    call = channel.unary_stream(
        "/TestService/Stream",
        request_serializer=lambda x: x,
        response_deserializer=lambda x: x,
    )

    responses = []
    try:
        async for response in call(b"request", timeout=5):
            responses.append(response)
    except asyncio.CancelledError:
        pass

    # Verify we received all responses
    assert responses == [b"response-1", b"response-2", b"response-3"]

    # Verify request was published
    mock_client_app.publish.assert_called_once()


@pytest.mark.asyncio
async def test_channel_stream_unary_call(mock_client_app: AsyncMock) -> None:
    """Test that channel correctly sends streaming requests."""
    channel_factory = channel_module.ChannelFactory(local_app=mock_client_app)
    channel = channel_factory.new_channel("org/ns/server")

    # Create mock session
    mock_session_info = MagicMock()
    mock_session_info.id = 1
    mock_session_info.metadata = {}
    mock_client_app.create_session.return_value = mock_session_info

    # Mock receiving the final response with proper code
    response_metadata = {"code": str(code_pb2.OK)}
    mock_client_app.receive.return_value = (
        MagicMock(metadata=response_metadata),
        b"aggregated",
    )

    # Make the call with streaming requests
    call = channel.stream_unary(
        "/TestService/Aggregate",
        request_serializer=lambda x: x,
        response_deserializer=lambda x: x,
    )

    async def request_stream() -> AsyncIterator[bytes]:
        for chunk in [b"chunk-1", b"chunk-2", b"chunk-3"]:
            yield chunk

    response = await call(request_stream(), timeout=5)

    # Verify multiple requests were published (3 data + 1 end-of-stream)
    assert mock_client_app.publish.call_count == 4

    # Check each request chunk
    calls = mock_client_app.publish.call_args_list
    assert calls[0][0][1] == b"chunk-1"
    assert calls[1][0][1] == b"chunk-2"
    assert calls[2][0][1] == b"chunk-3"
    # Fourth call is end-of-stream marker
    assert calls[3][0][1] == b""

    # Verify final response
    assert response == b"aggregated"


@pytest.mark.asyncio
async def test_server_registration_and_handler_lookup(
    mock_server_app: AsyncMock,
) -> None:
    """Test that handlers are correctly registered and can be looked up."""
    server = server_module.Server(local_app=mock_server_app)

    async def handler1(request: bytes, context: Context) -> bytes:
        return request

    async def handler2(request: bytes, context: Context) -> bytes:
        return request

    rpc_handler1 = unary_unary_rpc_method_handler(
        handler1,
        request_deserializer=lambda x: x,
        response_serializer=lambda x: x,
    )

    rpc_handler2 = unary_unary_rpc_method_handler(
        handler2,
        request_deserializer=lambda x: x,
        response_serializer=lambda x: x,
    )

    # Register multiple handlers
    server.register_rpc("Service1", "Method1", rpc_handler1)
    server.register_rpc("Service2", "Method2", rpc_handler2)

    # Verify handlers are stored correctly
    assert len(server.handlers) == 2
    assert any(
        sm.service == "Service1" and sm.method == "Method1" for sm in server.handlers
    )
    assert any(
        sm.service == "Service2" and sm.method == "Method2" for sm in server.handlers
    )


@pytest.mark.asyncio
async def test_channel_timeout_metadata(mock_client_app: AsyncMock) -> None:
    """Test that timeout is correctly added to request metadata."""
    channel_factory = channel_module.ChannelFactory(local_app=mock_client_app)
    channel = channel_factory.new_channel("org/ns/server")

    mock_session_info = MagicMock()
    mock_session_info.id = 1
    mock_session_info.metadata = {}
    mock_client_app.create_session.return_value = mock_session_info

    response_metadata = {"code": str(code_pb2.OK)}
    mock_client_app.receive.return_value = (
        MagicMock(metadata=response_metadata),
        b"response",
    )

    call = channel.unary_unary(
        "/Service/Method",
        request_serializer=lambda x: x,
        response_deserializer=lambda x: x,
    )

    # Make call with specific timeout
    await call(b"request", timeout=10)

    # Verify timeout was added to metadata
    publish_call = mock_client_app.publish.call_args
    metadata = publish_call[1]["metadata"]

    assert common_module.DEADLINE_KEY in metadata
    # Deadline should be a timestamp (current time + timeout)
    # Just verify it exists and is a reasonable value
    deadline = float(metadata[common_module.DEADLINE_KEY])
    assert deadline > 0  # Should be a positive timestamp
