# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
from collections.abc import AsyncIterable, AsyncIterator
from dataclasses import dataclass
from types import SimpleNamespace

import pytest

from slimrpc import channel as channel_module
from slimrpc import common as common_module
from slimrpc import server as server_module
from slimrpc.context import Context
from slimrpc.rpc import (
    stream_unary_rpc_method_handler,
    unary_stream_rpc_method_handler,
    unary_unary_rpc_method_handler,
)


class FakePyName:
    # Lightweight replacement for slim_bindings.PyName used by the harness
    def __init__(
        self,
        organization: str,
        namespace: str,
        application: str,
        identifier: str | None = None,
    ) -> None:
        self._components = (organization, namespace, application)
        self.id = identifier or f"{organization}-{namespace}-{application}"

    def components_strings(self) -> list[str]:
        return list(self._components)

    def __str__(self) -> str:
        return "/".join(self._components)

    def __hash__(self) -> int:
        return hash((self._components, self.id))

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, FakePyName):
            return False
        return self._components == other._components and self.id == other.id


@dataclass
class FakePySessionInfo:
    # Session metadata exchanged between fake client and server
    id: int
    source_name: FakePyName
    destination_name: FakePyName
    metadata: dict[str, str]
    payload_type: str = "binary"


class FakePySessionConfiguration:
    class FireAndForget:  # noqa: D401 - mimic slim bindings configuration type
        # Accepts arbitrary keyword arguments like the real bindings object
        def __init__(self, **_: object) -> None:
            return


@dataclass
class _SessionData:
    # Tracks delivery queues and routing info for a single fake session
    session_id: int
    client_name: FakePyName
    destination: FakePyName
    subscription: FakePyName
    request_queue: asyncio.Queue[tuple[FakePySessionInfo, bytes]]
    response_queue: asyncio.Queue[tuple[FakePySessionInfo, bytes]]
    handshake_sent: bool = False


class FakeTransport:
    # In-memory message bus that mimics slim_bindings transport behaviour
    def __init__(self) -> None:
        self._session_counter = 0
        self._sessions: dict[int, _SessionData] = {}
        self._incoming_sessions: asyncio.Queue[tuple[FakePySessionInfo, bytes]] = (
            asyncio.Queue()
        )
        self._subscription_map: dict[FakePyName, FakePyName] = {}

    def register_subscription(self, base: FakePyName, clone: FakePyName) -> None:
        # Remember mapping between canonical subscription and cloned target
        self._subscription_map[base] = clone

    def resolve_destination(self, base: FakePyName) -> FakePyName:
        # Resolve a routed destination to the clone when present
        return self._subscription_map.get(base, base)

    async def create_session(
        self, app: "FakeSlimApp", route: FakePyName
    ) -> FakePySessionInfo:
        # Create a new session record and enqueue handshake for server side
        self._session_counter += 1
        session_id = self._session_counter
        subscription = self.resolve_destination(route)
        session = _SessionData(
            session_id=session_id,
            client_name=app.local_name,
            destination=route,
            subscription=subscription,
            request_queue=asyncio.Queue(),
            response_queue=asyncio.Queue(),
        )
        self._sessions[session_id] = session
        return FakePySessionInfo(
            id=session_id,
            source_name=app.local_name,
            destination_name=route,
            metadata={},
        )

    async def publish(
        self,
        app: "FakeSlimApp",
        session_info: FakePySessionInfo,
        payload: bytes,
        dest: FakePyName,
        metadata: dict[str, str],
    ) -> None:
        # Deliver client payloads to server queues and emit handshake once
        session = self._sessions[session_info.id]
        subscription = self.resolve_destination(dest)

        if not session.handshake_sent:
            session.handshake_sent = True
            handshake = FakePySessionInfo(
                id=session.session_id,
                source_name=app.local_name,
                destination_name=subscription,
                metadata={},
            )
            await self._incoming_sessions.put((handshake, b""))

        request_info = FakePySessionInfo(
            id=session.session_id,
            source_name=app.local_name,
            destination_name=subscription,
            metadata=dict(metadata),
        )
        await session.request_queue.put((request_info, payload))

    async def receive(
        self, app: "FakeSlimApp", session_id: int | None
    ) -> tuple[FakePySessionInfo, bytes]:
        # Provide queued handshake/request to server or response to client
        if session_id is None:
            return await self._incoming_sessions.get()

        session = self._sessions[session_id]
        if app.role == "server":
            return await session.request_queue.get()
        return await session.response_queue.get()

    async def publish_to(
        self,
        app: "FakeSlimApp",
        session_info: FakePySessionInfo,
        payload: bytes,
        metadata: dict[str, str],
    ) -> None:
        # Push server response bytes back to the originating client session
        session = self._sessions[session_info.id]
        response_info = FakePySessionInfo(
            id=session.session_id,
            source_name=app.local_name,
            destination_name=session.client_name,
            metadata=dict(metadata),
        )
        await session.response_queue.put((response_info, payload))

    async def delete_session(self, session_id: int) -> None:
        # Leave queues intact; channel drains them during shutdown
        return


class FakeSlimApp:
    # Simplified SLIM app façade understood by channel and server modules
    def __init__(
        self, transport: FakeTransport, role: str, identity: FakePyName
    ) -> None:
        self.transport = transport
        self.role = role
        self.local_name = identity
        self._route: FakePyName | None = None

    async def __aenter__(self) -> "FakeSlimApp":
        return self

    async def __aexit__(self, *_: object) -> None:
        return

    async def set_route(self, route: FakePyName) -> None:
        self._route = route

    async def create_session(
        self, _: FakePySessionConfiguration.FireAndForget
    ) -> FakePySessionInfo:
        assert self._route is not None
        return await self.transport.create_session(self, self._route)

    async def publish(
        self,
        session_info: FakePySessionInfo,
        payload: bytes,
        dest: FakePyName,
        metadata: dict[str, str],
    ) -> None:
        await self.transport.publish(self, session_info, payload, dest, metadata)

    async def receive(
        self,
        session: int | None = None,
    ) -> tuple[FakePySessionInfo, bytes]:
        return await self.transport.receive(self, session)

    async def publish_to(
        self,
        session_info: FakePySessionInfo,
        payload: bytes,
        metadata: dict[str, str],
    ) -> None:
        await self.transport.publish_to(self, session_info, payload, metadata)

    async def delete_session(self, session_id: int) -> None:
        await self.transport.delete_session(session_id)

    async def subscribe(self, _: FakePyName) -> None:
        # Subscription handling delegated to FakeTransport
        return

    def get_id(self) -> str:
        return str(self.local_name)


async def _prepare_server(
    server: server_module.Server,
    local_app: FakeSlimApp,
    transport: FakeTransport,
) -> None:
    # Register handlers and subscription clones for the fake transport
    await local_app.subscribe(local_app.local_name)
    server._pyname_to_handler = {}  # type: ignore[attr-defined]
    for service_method, handler in server.handlers.items():
        subscription_name = common_module.handler_name_to_pyname(
            local_app.local_name,
            service_method.service,
            service_method.method,
        )
        components = subscription_name.components_strings()
        clone = FakePyName(
            components[0],
            components[1],
            components[2],
            local_app.local_name.id,
        )
        transport.register_subscription(subscription_name, clone)
        await local_app.subscribe(clone)
        server._pyname_to_handler[subscription_name] = handler  # type: ignore[attr-defined]
        server._pyname_to_handler[clone] = handler  # type: ignore[attr-defined]


def _setup_fake_environment(
    monkeypatch: pytest.MonkeyPatch,
) -> tuple[
    FakeTransport,
    FakeSlimApp,
    FakeSlimApp,
    server_module.Server,
    channel_module.ChannelFactory,
]:
    # Monkeypatch bindings and return fully wired fake transport/apps
    fake_bindings = SimpleNamespace(
        PyName=FakePyName,
        PySessionInfo=FakePySessionInfo,
        PySessionConfiguration=FakePySessionConfiguration,
    )
    monkeypatch.setattr(common_module, "slim_bindings", fake_bindings)
    monkeypatch.setattr(channel_module, "slim_bindings", fake_bindings)
    monkeypatch.setattr(server_module, "slim_bindings", fake_bindings)

    transport = FakeTransport()
    server_identity = FakePyName("org", "ns", "server", "server-instance")
    client_identity = FakePyName("org", "ns", "client", "client-instance")
    server_app = FakeSlimApp(transport, role="server", identity=server_identity)
    client_app = FakeSlimApp(transport, role="client", identity=client_identity)

    server = server_module.Server(local_app=server_app)
    channel_factory = channel_module.ChannelFactory(local_app=client_app)
    return transport, server_app, client_app, server, channel_factory


@pytest.mark.asyncio
async def test_unary_unary_round_trip(monkeypatch: pytest.MonkeyPatch) -> None:
    # Verify unary request/response works through the fake transport
    (
        transport,
        server_app,
        _client_app,
        server,
        channel_factory,
    ) = _setup_fake_environment(monkeypatch)

    async def behaviour(request: bytes, context: Context) -> bytes:
        deadline = (
            context.metadata.get(common_module.DEADLINE_KEY, "")
            if context.metadata
            else ""
        )
        assert deadline
        return request.upper()

    handler = unary_unary_rpc_method_handler(
        behaviour,
        request_deserializer=lambda payload: payload,
        response_serializer=lambda payload: payload,
    )
    server.register_rpc("service", "Ping", handler)
    await _prepare_server(server, server_app, transport)

    channel = channel_factory.new_channel("org/ns/server")
    call = channel.unary_unary(
        "/service/Ping",
        request_serializer=lambda value: value,
        response_deserializer=lambda value: value,
    )

    async def server_worker() -> None:
        session_info, _ = await server_app.receive()
        await server.handle_session(session_info, server_app)

    worker = asyncio.create_task(server_worker())
    response = await call(b"hello", timeout=5)
    await worker

    assert response == b"HELLO"


@pytest.mark.asyncio
async def test_unary_stream_round_trip(monkeypatch: pytest.MonkeyPatch) -> None:
    # Verify unary request yields a stream of responses to the client
    (
        transport,
        server_app,
        _client_app,
        server,
        channel_factory,
    ) = _setup_fake_environment(monkeypatch)

    async def behaviour(request: bytes, context: Context) -> AsyncIterator[bytes]:
        deadline = (
            context.metadata.get(common_module.DEADLINE_KEY, "")
            if context.metadata
            else ""
        )
        assert deadline
        assert request == b"hello"
        for index in range(3):
            yield request + f"-{index}".encode()

    handler = unary_stream_rpc_method_handler(
        behaviour,
        request_deserializer=lambda payload: payload,
        response_serializer=lambda payload: payload,
    )
    server.register_rpc("service", "PingStream", handler)
    await _prepare_server(server, server_app, transport)

    channel = channel_factory.new_channel("org/ns/server")
    call = channel.unary_stream(
        "/service/PingStream",
        request_serializer=lambda value: value,
        response_deserializer=lambda value: value,
    )

    async def server_worker() -> None:
        session_info, _ = await server_app.receive()
        await server.handle_session(session_info, server_app)

    worker = asyncio.create_task(server_worker())

    responses: list[bytes] = []
    async for payload in call(b"hello", timeout=5):
        responses.append(payload)

    await worker

    assert responses == [b"hello-0", b"hello-1", b"hello-2"]


@pytest.mark.asyncio
async def test_stream_unary_round_trip(monkeypatch: pytest.MonkeyPatch) -> None:
    # Verify streaming requests are aggregated into a single response
    (
        transport,
        server_app,
        _client_app,
        server,
        channel_factory,
    ) = _setup_fake_environment(monkeypatch)

    async def behaviour(
        request_iterator: AsyncIterable[tuple[bytes, Context]], context: Context
    ) -> bytes:
        chunks: list[bytes] = []
        deadlines: list[str] = []
        async for chunk, chunk_context in request_iterator:
            assert chunk_context.metadata
            deadlines.append(chunk_context.metadata.get(common_module.DEADLINE_KEY, ""))
            chunks.append(chunk)
        assert all(deadlines)
        return b"|".join(chunks)

    handler = stream_unary_rpc_method_handler(
        behaviour,
        request_deserializer=lambda payload: payload,
        response_serializer=lambda payload: payload,
    )
    server.register_rpc("service", "Aggregate", handler)
    await _prepare_server(server, server_app, transport)

    channel = channel_factory.new_channel("org/ns/server")
    call = channel.stream_unary(
        "/service/Aggregate",
        request_serializer=lambda value: value,
        response_deserializer=lambda value: value,
    )

    async def server_worker() -> None:
        session_info, _ = await server_app.receive()
        await server.handle_session(session_info, server_app)

    worker = asyncio.create_task(server_worker())

    async def request_stream() -> AsyncIterator[bytes]:
        for chunk in [b"first", b"second", b"third"]:
            yield chunk

    response = await call(request_stream(), timeout=5)
    await worker

    assert response == b"first|second|third"
