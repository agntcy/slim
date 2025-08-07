# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime
import logging
import sys
from time import time
from collections.abc import AsyncIterable, Callable

import slim_bindings
from google.rpc import code_pb2

from srpc.common import (
    DEADLINE_KEY,
    MAX_TIMEOUT,
    create_local_app,
    service_and_method_to_pyname,
    split_id,
)

logging.basicConfig(level=logging.DEBUG)
logger = logging.getLogger(__name__)


class Channel:
    def __init__(
        self,
        local: str,
        slim: dict,
        remote: str,
        enable_opentelemetry: bool = False,
        shared_secret: str | None = None,
    ):
        self.local = split_id(local)
        self.slim = slim
        self.enable_opentelemetry = enable_opentelemetry
        self.shared_secret = shared_secret
        self.remote = split_id(remote)
        self.local_app: slim_bindings.Slim | None = None
        self._prepare_task = asyncio.get_running_loop().create_task(
            self._prepare_channel()
        )

    async def _prepare_channel(self):
        # Create local SLIM instance
        self.local_app = await create_local_app(
            self.local,
            self.slim,
            enable_opentelemetry=self.enable_opentelemetry,
            shared_secret=self.shared_secret,
        )

        # Start receiving messages
        await self.local_app.__aenter__()

    def close(self) -> None:
        """
        Close the channel.
        """
        if self.local_app is not None:
            self.local_app.__aexit__(None, None, None)

    async def _common_setup(self, method: str, metadata: dict | None = None):
        service_name = service_and_method_to_pyname(self.remote, method)

        assert self.local_app is not None
        await self.local_app.set_route(
            service_name,
        )

        # Create a session
        session = await self.local_app.create_session(
            slim_bindings.PySessionConfiguration.FireAndForget(
                max_retries=10,
                timeout=datetime.timedelta(seconds=1),
                sticky=True,
            )
        )

        return service_name, session, metadata or {}

    async def _delete_session(self, session: slim_bindings.PySessionInfo):
        assert self.local_app is not None
        await self.local_app.delete_session(session.id)

    async def _send_unary(
        self,
        request,
        session,
        service_name,
        metadata,
        request_serializer,
        deadline: int,
    ):
        # Add deadline to metadata
        metadata[DEADLINE_KEY] = str(deadline)

        # Send the request
        request_bytes = request_serializer(request)
        assert self.local_app is not None
        await self.local_app.publish(
            session,
            request_bytes,
            dest=service_name,
            metadata=metadata,
        )

    async def _send_stream(
        self,
        request_stream,
        session: slim_bindings.PySessionInfo,
        service_name: slim_bindings.PyName,
        metadata: dict,
        request_serializer,
        deadline: int,
    ):
        assert self.local_app is not None

        # Add deadline to metadata
        metadata[DEADLINE_KEY] = str(deadline)

        # Send requests
        async for request in request_stream:
            request_bytes = request_serializer(request)
            await self.local_app.publish(
                session,
                request_bytes,
                dest=service_name,
                metadata=metadata,
            )

        # Send enf of streaming message
        await self.local_app.publish(
            session,
            b"",
            dest=service_name,
            metadata={**metadata, "code": str(code_pb2.OK)},
        )

    async def _receive_unary(self, session, response_deserializer, deadline: int):
        # Wait for the response
        assert self.local_app is not None

        async with asyncio.timeout_at(deadline):
            session_recv, response_bytes = await self.local_app.receive(
                session=session.id,
            )
            response = response_deserializer(response_bytes)
            return session_recv, response

    async def _receive_stream(self, session, response_deserializer, deadline: int):
        # Wait for the responses

        async def generator():
            try:
                while True:
                    session_recv, response_bytes = await self.local_app.receive(
                        session=session.id,
                    )

                    if (
                        session_recv.metadata.get("code") == str(code_pb2.OK)
                        and not response_bytes
                    ):
                        logger.debug("end of stream received")
                        break

                    response = response_deserializer(response_bytes)
                    yield response
            except Exception as e:
                logger.error(f"error receiving messages: {e}")
                raise

        async with asyncio.timeout_at(deadline):
            async for response in generator():
                yield response

    def stream_stream(
        self,
        method: str,
        request_serializer: Callable = lambda x: x,
        response_deserializer: Callable = lambda x: x,
    ):
        async def call_stream_stream(
            request_stream: AsyncIterable,
            timeout=MAX_TIMEOUT,
            metadata=None,
            credentials=None,
            wait_for_ready=None,
            compression=None,
        ):
            try:
                await self._prepare_task
                service_name, session, metadata = await self._common_setup(method)

                # Send the requests
                await self._send_stream(
                    request_stream,
                    session,
                    service_name,
                    metadata,
                    request_serializer,
                    _compute_deadline(timeout),
                )

            # Send end of streaming message
            await self.local_app.publish(
                session,
                b"",
                dest=service_name,
                metadata={"code": str(code_pb2.OK)},
            )

            # Wait for the responses
            async def generator():
                try:
                    while True:
                        session_recv, response_bytes = await self.local_app.receive(
                            session=session.id,
                        )

                        if (
                            session_recv.metadata.get("code") == str(code_pb2.OK)
                            and not response_bytes
                        ):
                            logger.debug("end of stream received")
                            break

                        response = response_deserializer(response_bytes)
                        yield response
                except Exception as e:
                    logger.error(f"error receiving messages: {e}")
                    raise

            async for response in generator():
                yield response

        return call_stream_stream

    def stream_unary(
        self,
        method: str,
        request_serializer: Callable = lambda x: x,
        response_deserializer: Callable = lambda x: x,
    ):
        async def call_stream_unary(
            request_stream: AsyncIterable,
            timeout=MAX_TIMEOUT,
            metadata=None,
            credentials=None,
            wait_for_ready=None,
            compression=None,
        ):
            try:
                await self._prepare_task
                service_name, session, metadata = await self._common_setup(method)

                # Send the requests
                await self._send_stream(
                    request_stream,
                    session,
                    service_name,
                    metadata,
                    request_serializer,
                    _compute_deadline(timeout),
                )

                # Wait for response
                _, ret = await self._receive_unary(
                    session, response_deserializer, _compute_deadline(timeout)
                )
            finally:
                await self._delete_session(session)

        return call_stream_unary

    def unary_stream(
        self,
        method: str,
        request_serializer: Callable = lambda x: x,
        response_deserializer: Callable = lambda x: x,
    ):
        async def call_unary_stream(
            request,
            timeout=MAX_TIMEOUT,
            metadata=None,
            credentials=None,
            wait_for_ready=None,
            compression=None,
        ):
            try:
                await self._prepare_task
                service_name, session, metadata = await self._common_setup(method)

                # Send the request
                await self._send_unary(
                    request,
                    session,
                    service_name,
                    metadata,
                    request_serializer,
                    _compute_deadline(timeout),
                )

                # Wait for the responses
                async for response in self._receive_stream(
                    session, response_deserializer, _compute_deadline(timeout)
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
    ):
        async def call_unary_unary(
            request,
            timeout=MAX_TIMEOUT,
            metadata=None,
            credentials=None,
            wait_for_ready=None,
            compression=None,
        ):
            try:
                await self._prepare_task
                service_name, session, metadata = await self._common_setup(method)

                # Send request
                await self._send_unary(
                    request,
                    session,
                    service_name,
                    metadata,
                    request_serializer,
                    _compute_deadline(timeout),
                )

                # Wait for the response
                _, ret = await self._receive_unary(
                    session, response_deserializer, _compute_deadline(timeout)
                )

                return ret
            finally:
                await self._delete_session(session)

        return call_unary_unary


def _compute_deadline(timeout: int) -> float:
    return asyncio.get_running_loop().time() + float(timeout)
