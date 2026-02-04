# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import datetime
import logging
import sys
import time
from asyncio import get_running_loop
from collections.abc import AsyncGenerator, AsyncIterable, Callable
from typing import Any

if sys.version_info >= (3, 11):
    from asyncio import timeout_at as asyncio_timeout_at
else:
    from async_timeout import timeout_at as asyncio_timeout_at

import slim_bindings
from google.rpc import code_pb2, status_pb2

from slimrpc.common import (
    DEADLINE_KEY,
    MAX_TIMEOUT,
    RequestType,
    ResponseType,
    SLIMAppConfig,
    create_local_app,
    service_and_method_to_pyname,
    split_id,
)
from slimrpc.rpc import SRPCResponseError

logger = logging.getLogger(__name__)


class Channel:
    def __init__(
        self,
        remote: str,
        local_app: slim_bindings.Slim,
    ) -> None:
        self.remote = split_id(remote)
        self.local_app = local_app

    @classmethod
    async def from_slim_app_config(
        cls, remote: str, slim_app_config: SLIMAppConfig
    ) -> "Channel":
        local_app = await create_local_app(slim_app_config)
        return cls(remote=remote, local_app=local_app)

    async def _common_setup(
        self, method: str, metadata: dict[str, str] | None = None
    ) -> tuple[slim_bindings.Name, slim_bindings.Session, dict[str, str]]:
        service_name = service_and_method_to_pyname(self.remote, method)

        await self.local_app.set_route(
            service_name,
        )

        logger.info(f"creating session for service {service_name}")

        # Create a session using PointToPoint configuration
        session, ack = await self.local_app.create_session(
            destination=service_name,
            session_config=slim_bindings.SessionConfiguration.PointToPoint(
                max_retries=10,
                timeout=datetime.timedelta(seconds=1),
            ),
        )
        await ack

        return service_name, session, metadata or {}

    async def _delete_session(self, session: slim_bindings.Session) -> None:
        try:
            await self.local_app.delete_session(session)
        except Exception as e:
            logger.warn(f"Error deleting session: {e}")

    async def _send_unary(
        self,
        request: RequestType,
        session: slim_bindings.Session,
        service_name: slim_bindings.Name,
        metadata: dict[str, str],
        request_serializer: Callable,
        deadline: float,
    ) -> None:
        # Add deadline to metadata
        metadata[DEADLINE_KEY] = str(deadline)

        # Send the request
        request_bytes = request_serializer(request)
        ack = await session.publish(
            request_bytes,
            metadata=metadata,
        )
        await ack

    async def _send_stream(
        self,
        request_stream: AsyncIterable,
        session: slim_bindings.Session,
        service_name: slim_bindings.Name,
        metadata: dict[str, str],
        request_serializer: Callable,
        deadline: float,
    ) -> None:
        # Add deadline to metadata
        metadata[DEADLINE_KEY] = str(deadline)

        # Send requests
        async for request in request_stream:
            request_bytes = request_serializer(request)
            ack = await session.publish(
                request_bytes,
                metadata=metadata,
            )
            await ack

        # Send end of streaming message
        ack = await session.publish(
            b"",
            metadata={**metadata, "code": str(code_pb2.OK)},
        )
        await ack

    async def _receive_unary(
        self,
        session: slim_bindings.Session,
        response_deserializer: Callable,
        deadline: float,
    ) -> tuple[slim_bindings.MessageContext, Any]:
        # Wait for the response
        async with asyncio_timeout_at(
            _compute_loop_deadline_from_real_deadline(deadline)
        ):
            msg_ctx, response_bytes = await session.get_message()

            code = msg_ctx.metadata.get("code")
            if code != str(code_pb2.OK):
                status = status_pb2.Status.FromString(response_bytes)
                raise SRPCResponseError(status.code, status.message, status.details)

            response = response_deserializer(response_bytes)
            return msg_ctx, response

    async def _receive_stream(
        self,
        session: slim_bindings.Session,
        response_deserializer: Callable,
        deadline: float,
    ) -> AsyncIterable:
        # Wait for the responses
        async def generator() -> AsyncIterable:
            try:
                while True:
                    msg_ctx, response_bytes = await session.get_message()

                    code = msg_ctx.metadata.get("code")
                    if code != str(code_pb2.OK):
                        status = status_pb2.Status.FromString(response_bytes)
                        raise SRPCResponseError(
                            status.code, status.message, status.details
                        )

                    if not response_bytes:
                        logger.debug("end of stream received")
                        break

                    response = response_deserializer(response_bytes)
                    yield response
            except SRPCResponseError:
                raise
            except Exception as e:
                logger.error(f"error receiving messages: {e}")
                raise

        async with asyncio_timeout_at(
            _compute_loop_deadline_from_real_deadline(deadline)
        ):
            async for response in generator():
                yield response

    def stream_stream(
        self,
        method: str,
        request_serializer: Callable = lambda x: x,
        response_deserializer: Callable = lambda x: x,
    ) -> Callable:
        async def call_stream_stream(
            request_stream: AsyncIterable,
            timeout: int = MAX_TIMEOUT,
            metadata: dict | None = None,
        ) -> AsyncIterable:
            try:
                service_name, session, metadata = await self._common_setup(
                    method, metadata
                )

                deadline = _compute_real_deadline(timeout)

                # Send the requests
                await self._send_stream(
                    request_stream,
                    session,
                    service_name,
                    metadata,
                    request_serializer,
                    deadline,
                )

                # Wait for the responses
                async for response in self._receive_stream(
                    session,
                    response_deserializer,
                    deadline,
                ):
                    yield response
            finally:
                await self._delete_session(session)

        return call_stream_stream

    def stream_unary(
        self,
        method: str,
        request_serializer: Callable = lambda x: x,
        response_deserializer: Callable = lambda x: x,
    ) -> Callable:
        async def call_stream_unary(
            request_stream: AsyncIterable,
            timeout: int = MAX_TIMEOUT,
            metadata: dict | None = None,
        ) -> ResponseType:
            try:
                service_name, session, metadata = await self._common_setup(
                    method, metadata
                )

                deadline = _compute_real_deadline(timeout)

                # Send the requests
                await self._send_stream(
                    request_stream,
                    session,
                    service_name,
                    metadata,
                    request_serializer,
                    deadline,
                )

                # Wait for response
                _, ret = await self._receive_unary(
                    session,
                    response_deserializer,
                    deadline,
                )

                return ret
            finally:
                await self._delete_session(session)

        return call_stream_unary

    def unary_stream(
        self,
        method: str,
        request_serializer: Callable = lambda x: x,
        response_deserializer: Callable = lambda x: x,
    ) -> Callable:
        async def call_unary_stream(
            request: RequestType,
            timeout: int = MAX_TIMEOUT,
            metadata: dict[str, str] | None = None,
        ) -> AsyncGenerator:
            try:
                service_name, session, metadata = await self._common_setup(
                    method, metadata
                )

                deadline = _compute_real_deadline(timeout)

                # Send the request
                await self._send_unary(
                    request,
                    session,
                    service_name,
                    metadata,
                    request_serializer,
                    deadline,
                )

                # Wait for the responses
                async for response in self._receive_stream(
                    session,
                    response_deserializer,
                    deadline,
                ):
                    yield response
            finally:
                await self._delete_session(session)

        return call_unary_stream

    def unary_unary(
        self,
        method: str,
        request_serializer: Callable = lambda x: x,
        response_deserializer: Callable = lambda x: x,
    ) -> Callable:
        async def call_unary_unary(
            request: RequestType,
            timeout: int = MAX_TIMEOUT,
            metadata: dict[str, str] | None = None,
        ) -> ResponseType:
            try:
                service_name, session, metadata = await self._common_setup(
                    method, metadata
                )

                deadline = _compute_real_deadline(timeout)

                # Send request
                await self._send_unary(
                    request,
                    session,
                    service_name,
                    metadata,
                    request_serializer,
                    deadline,
                )

                # Wait for the response
                _, ret = await self._receive_unary(
                    session,
                    response_deserializer,
                    deadline,
                )

                return ret
            finally:
                await self._delete_session(session)

        return call_unary_unary


def _compute_real_deadline(timeout: int) -> float:
    return time.time() + float(timeout)


def _compute_loop_deadline_from_real_deadline(real_deadline: float) -> float:
    return get_running_loop().time() + (real_deadline - time.time())
