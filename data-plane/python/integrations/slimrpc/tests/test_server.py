# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from collections.abc import Iterable
from types import SimpleNamespace
from typing import cast

import pytest
from google.rpc import code_pb2

from slimrpc.context import Context
from slimrpc.server import request_generator

OK_CODE: int = cast(int, code_pb2.OK)


class FakeLocalApp:
    def __init__(self, responses: Iterable[tuple[SimpleNamespace, bytes]]) -> None:
        self._responses: list[tuple[SimpleNamespace, bytes]] = list(responses)
        self.calls: list[int] = []

    async def receive(self, session: int) -> tuple[SimpleNamespace, bytes]:
        self.calls.append(session)
        return self._responses.pop(0)


def _session(metadata: dict[str, str]) -> SimpleNamespace:
    return SimpleNamespace(
        id=42,
        metadata=metadata,
        source_name="org/ns/source",
        destination_name="org/ns/destination",
        payload_type="binary",
    )


@pytest.mark.asyncio
async def test_request_generator_yields_until_terminator() -> None:
    responses: list[tuple[SimpleNamespace, bytes]] = [
        (_session({"seq": "1"}), b"first"),
        (_session({"seq": "2"}), b"second"),
        (_session({"code": str(OK_CODE)}), b""),
    ]
    local_app = FakeLocalApp(responses)
    session_info = _session({})

    collected: list[tuple[str, Context]] = []
    async for request, ctx in request_generator(
        local_app, lambda data: data.decode("utf-8"), session_info
    ):
        collected.append((request, ctx))

    assert [request for request, _ in collected] == ["first", "second"]
    assert all(isinstance(ctx, Context) for _, ctx in collected)
    assert local_app.calls == [session_info.id, session_info.id, session_info.id]


@pytest.mark.asyncio
async def test_request_generator_propagates_deserializer_errors() -> None:
    responses = [(_session({}), b"boom")]
    local_app = FakeLocalApp(responses)
    session_info = _session({})

    def failing_deserializer(_: bytes) -> str:
        raise ValueError("deserialize error")

    async def consume() -> None:
        async for _ in request_generator(local_app, failing_deserializer, session_info):
            pass

    with pytest.raises(ValueError):
        await consume()
