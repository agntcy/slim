# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime
import logging
from collections.abc import AsyncIterable

import slim_bindings
from google.rpc import code_pb2

from srpc.common import (
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
        self.handlers = {}
        self.remote = split_id(remote)
        self.local_app: slim_bindings.Slim = None
        self.prepare_task = asyncio.get_running_loop().create_task(
            self.prepare_channel()
        )

    async def prepare_channel(self):
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

    async def common_setup(self, method: str):
        service_name = service_and_method_to_pyname(self.remote, method)

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

        return service_name, session

    async def send_unary(
        self, request, session, service_name, metadata, request_serializer
    ):
        # Send the request
        request_bytes = request_serializer(request)
        await self.local_app.publish(
            session,
            request_bytes,
            dest=service_name,
            metadata=metadata,
        )

    async def send_stream(
        self, request_stream, session, service_name, metadata, request_serializer
    ):
        # Send the request
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
            metadata={"code": str(code_pb2.OK)},
        )

    async def receive_unary(self, session, response_deserializer):
        # Wait for the response
        session_recv, response_bytes = await self.local_app.receive(
            session=session.id,
        )

        response = response_deserializer(response_bytes)

        return session_recv, response

    async def receive_stream(self, session, response_deserializer):
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

    def stream_stream(
        self,
        method: str,
        request_serializer: callable = lambda x: x,
        response_deserializer: callable = lambda x: x,
    ):
        async def call_stream_stream(
            request_stream: AsyncIterable,
            timeout=None,
            metadata=None,
            credentials=None,
            wait_for_ready=None,
            compression=None,
        ):
            await self.prepare_task
            service_name, session = await self.common_setup(method)

            # Send the request
            async for request in request_stream:
                request_bytes = request_serializer(request)
                await self.local_app.publish(
                    session,
                    request_bytes,
                    dest=service_name,
                    metadata=metadata,
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
        request_serializer: callable = lambda x: x,
        response_deserializer: callable = lambda x: x,
    ):
        async def call_stream_unary(
            request_stream: AsyncIterable,
            timeout=None,
            metadata=None,
            credentials=None,
            wait_for_ready=None,
            compression=None,
        ):
            await self.prepare_task
            service_name, session = await self.common_setup(method)

            # Send the requests
            await self.send_stream(
                request_stream, session, service_name, metadata, request_serializer
            )

            # Wait for response
            return await self.receive_unary(session, response_deserializer)

        return call_stream_unary

    def unary_stream(
        self,
        method: str,
        request_serializer: callable = lambda x: x,
        response_deserializer: callable = lambda x: x,
    ):
        async def call_unary_stream(
            request,
            timeout=None,
            metadata=None,
            credentials=None,
            wait_for_ready=None,
            compression=None,
        ):
            await self.prepare_task
            service_name, session = await self.common_setup(method)

            # Send the request
            await self.send_unary(
                request, session, service_name, metadata, request_serializer
            )

            # Wait for the responses
            async for response in self.receive_stream(session, response_deserializer):
                yield response

        return call_unary_stream

    def unary_unary(
        self,
        method: str,
        request_serializer: callable = lambda x: x,
        response_deserializer: callable = lambda x: x,
    ):
        async def call_unary_unary(
            request,
            timeout=None,
            metadata=None,
            credentials=None,
            wait_for_ready=None,
            compression=None,
        ):
            await self.prepare_task
            service_name, session = await self.common_setup(method)

            # Send request
            await self.send_unary(
                request, session, service_name, metadata, request_serializer
            )

            # Wait for the response
            session_recv, response = await self.receive_unary(
                session, response_deserializer
            )

            return response

        return call_unary_unary
