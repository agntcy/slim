# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import time
from unittest.mock import AsyncMock, MagicMock

import pytest
import slim_bindings
from freezegun import freeze_time
from google.rpc import code_pb2

from slimrpc import channel as channel_module
from slimrpc import common as common_module


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
@freeze_time("2024-01-01 12:00:00")
async def test_channel_deadline_is_future_timestamp(
    mock_client_app: AsyncMock,
) -> None:
    """Test that deadline is correctly computed as future timestamp."""
    channel_factory = channel_module.ChannelFactory(local_app=mock_client_app)
    channel = channel_factory.new_channel("org/ns/server")

    # Create mock session
    mock_session_info = MagicMock()
    mock_session_info.id = 1
    mock_session_info.metadata = {}
    mock_client_app.create_session.return_value = mock_session_info

    # Mock successful response
    mock_client_app.receive.return_value = (
        MagicMock(metadata={"code": str(code_pb2.OK)}),
        b"response",
    )

    call = channel.unary_unary(
        "/TestService/Echo",
        request_serializer=lambda x: x,
        response_deserializer=lambda x: x,
    )

    # Capture the current time
    start_time = time.time()

    await call(b"request", timeout=10)

    # Verify deadline was passed in metadata
    publish_call = mock_client_app.publish.call_args
    metadata = publish_call[1]["metadata"]

    assert common_module.DEADLINE_KEY in metadata
    deadline = float(metadata[common_module.DEADLINE_KEY])

    # Deadline should be approximately start_time + 10 seconds
    expected_deadline = start_time + 10
    assert abs(deadline - expected_deadline) < 0.5  # Allow 0.5s tolerance


@pytest.mark.asyncio
@freeze_time("2024-01-01 12:00:00")
async def test_deadline_computation_with_frozen_time(
    mock_client_app: AsyncMock,
) -> None:
    """Test deadline computation using frozen time for deterministic results."""
    # With frozen time, time.time() always returns the same value
    frozen_timestamp = time.time()
    assert frozen_timestamp == time.time()  # Verify time is frozen

    channel_factory = channel_module.ChannelFactory(local_app=mock_client_app)
    channel = channel_factory.new_channel("org/ns/server")

    mock_session_info = MagicMock()
    mock_session_info.id = 1
    mock_session_info.metadata = {}
    mock_client_app.create_session.return_value = mock_session_info

    mock_client_app.receive.return_value = (
        MagicMock(metadata={"code": str(code_pb2.OK)}),
        b"response",
    )

    call = channel.unary_unary(
        "/TestService/Echo",
        request_serializer=lambda x: x,
        response_deserializer=lambda x: x,
    )

    await call(b"request", timeout=5)

    # With frozen time, deadline should be exactly frozen_timestamp + 5
    publish_call = mock_client_app.publish.call_args
    metadata = publish_call[1]["metadata"]
    deadline = float(metadata[common_module.DEADLINE_KEY])

    assert deadline == frozen_timestamp + 5


@pytest.mark.asyncio
@freeze_time("2024-01-01 12:00:00")
async def test_multiple_requests_have_different_deadlines(
    mock_client_app: AsyncMock,
) -> None:
    """Test that different timeout values produce different deadlines."""
    channel_factory = channel_module.ChannelFactory(local_app=mock_client_app)
    channel = channel_factory.new_channel("org/ns/server")

    mock_session_info = MagicMock()
    mock_session_info.id = 1
    mock_session_info.metadata = {}
    mock_client_app.create_session.return_value = mock_session_info

    mock_client_app.receive.return_value = (
        MagicMock(metadata={"code": str(code_pb2.OK)}),
        b"response",
    )

    call = channel.unary_unary(
        "/TestService/Echo",
        request_serializer=lambda x: x,
        response_deserializer=lambda x: x,
    )

    frozen_timestamp = time.time()

    # Make first call with 5 second timeout
    await call(b"request1", timeout=5)
    first_deadline = float(
        mock_client_app.publish.call_args[1]["metadata"][common_module.DEADLINE_KEY]
    )

    # Make second call with 10 second timeout
    await call(b"request2", timeout=10)
    second_deadline = float(
        mock_client_app.publish.call_args[1]["metadata"][common_module.DEADLINE_KEY]
    )

    # Verify deadlines are different and correct
    assert first_deadline == frozen_timestamp + 5
    assert second_deadline == frozen_timestamp + 10
    assert second_deadline > first_deadline


@pytest.mark.asyncio
async def test_deadline_metadata_key_is_correct(mock_client_app: AsyncMock) -> None:
    """Test that the correct metadata key is used for deadline."""
    channel_factory = channel_module.ChannelFactory(local_app=mock_client_app)
    channel = channel_factory.new_channel("org/ns/server")

    mock_session_info = MagicMock()
    mock_session_info.id = 1
    mock_session_info.metadata = {}
    mock_client_app.create_session.return_value = mock_session_info

    mock_client_app.receive.return_value = (
        MagicMock(metadata={"code": str(code_pb2.OK)}),
        b"response",
    )

    call = channel.unary_unary(
        "/TestService/Echo",
        request_serializer=lambda x: x,
        response_deserializer=lambda x: x,
    )

    await call(b"request", timeout=5)

    # Verify the metadata key matches the constant
    publish_call = mock_client_app.publish.call_args
    metadata = publish_call[1]["metadata"]

    assert common_module.DEADLINE_KEY in metadata
    assert common_module.DEADLINE_KEY == "slimrpc-timeout"


@pytest.mark.asyncio
async def test_default_vs_custom_timeout(mock_client_app: AsyncMock) -> None:
    """Test that custom timeout overrides any defaults."""
    channel_factory = channel_module.ChannelFactory(local_app=mock_client_app)
    channel = channel_factory.new_channel("org/ns/server")

    mock_session_info = MagicMock()
    mock_session_info.id = 1
    mock_session_info.metadata = {}
    mock_client_app.create_session.return_value = mock_session_info

    mock_client_app.receive.return_value = (
        MagicMock(metadata={"code": str(code_pb2.OK)}),
        b"response",
    )

    call = channel.unary_unary(
        "/TestService/Echo",
        request_serializer=lambda x: x,
        response_deserializer=lambda x: x,
    )

    start_time = time.time()
    custom_timeout = 15

    await call(b"request", timeout=custom_timeout)

    # Verify custom timeout was used
    publish_call = mock_client_app.publish.call_args
    metadata = publish_call[1]["metadata"]
    deadline = float(metadata[common_module.DEADLINE_KEY])

    expected_deadline = start_time + custom_timeout
    # Allow 1 second tolerance for test execution time
    assert abs(deadline - expected_deadline) < 1.0


@pytest.mark.asyncio
async def test_stream_request_includes_deadline(mock_client_app: AsyncMock) -> None:
    """Test that streaming requests also include deadline in metadata."""
    channel_factory = channel_module.ChannelFactory(local_app=mock_client_app)
    channel = channel_factory.new_channel("org/ns/server")

    mock_session_info = MagicMock()
    mock_session_info.id = 1
    mock_session_info.metadata = {}
    mock_client_app.create_session.return_value = mock_session_info

    # Mock stream end
    mock_client_app.receive.return_value = (
        MagicMock(metadata={"code": str(code_pb2.OK)}),
        b"response",
    )

    call = channel.unary_stream(
        "/TestService/Stream",
        request_serializer=lambda x: x,
        response_deserializer=lambda x: x,
    )

    start_time = time.time()

    # Consume the stream
    responses = []
    async for response in call(b"request", timeout=7):
        responses.append(response)
        break  # Just get first response and exit

    # Verify deadline was included in the initial request
    first_publish_call = mock_client_app.publish.call_args_list[0]
    metadata = first_publish_call[1]["metadata"]

    assert common_module.DEADLINE_KEY in metadata
    deadline = float(metadata[common_module.DEADLINE_KEY])

    expected_deadline = start_time + 7
    assert abs(deadline - expected_deadline) < 1.0
