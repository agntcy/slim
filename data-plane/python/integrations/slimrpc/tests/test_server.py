# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from typing import cast
from unittest.mock import AsyncMock, MagicMock

import pytest
from google.rpc import code_pb2

from slimrpc.context import Context
from slimrpc.server import request_generator

OK_CODE: int = cast(int, code_pb2.OK)


def _create_session_info(session_id: int, metadata: dict[str, str]) -> MagicMock:
    """Create a mock session info object."""
    session_info = MagicMock()
    session_info.id = session_id
    session_info.metadata = metadata
    session_info.source_name = "org/ns/source"
    session_info.destination_name = "org/ns/destination"
    session_info.payload_type = "binary"
    return session_info


@pytest.mark.asyncio
async def test_request_generator_yields_until_terminator() -> None:
    """Test that request_generator yields requests until it receives terminator."""
    # Create mock app with side_effect to return multiple responses
    mock_app = AsyncMock()
    mock_app.receive.side_effect = [
        (_create_session_info(42, {"seq": "1"}), b"first"),
        (_create_session_info(42, {"seq": "2"}), b"second"),
        (_create_session_info(42, {"code": str(OK_CODE)}), b""),  # Terminator
    ]

    session_info = _create_session_info(42, {})

    # Collect all yielded requests
    collected: list[tuple[str, Context]] = []
    async for request, ctx in request_generator(
        mock_app, lambda data: data.decode("utf-8"), session_info
    ):
        collected.append((request, ctx))

    # Verify requests were deserialized correctly
    assert [request for request, _ in collected] == ["first", "second"]

    # Verify contexts are correct type
    assert all(isinstance(ctx, Context) for _, ctx in collected)

    # Verify receive was called three times (2 data + 1 terminator)
    assert mock_app.receive.call_count == 3

    # Verify all calls used the correct session ID
    for call_args in mock_app.receive.call_args_list:
        assert call_args[1]["session"] == session_info.id


@pytest.mark.asyncio
async def test_request_generator_propagates_deserializer_errors() -> None:
    """Test that request_generator propagates errors from deserializer."""
    # Create mock app that returns one response
    mock_app = AsyncMock()
    mock_app.receive.return_value = (_create_session_info(42, {}), b"boom")

    session_info = _create_session_info(42, {})

    def failing_deserializer(_: bytes) -> str:
        raise ValueError("deserialize error")

    # Verify that deserializer error propagates
    with pytest.raises(ValueError, match="deserialize error"):
        async for _ in request_generator(mock_app, failing_deserializer, session_info):
            pass

    # Verify receive was called once before error
    mock_app.receive.assert_called_once_with(session=session_info.id)
