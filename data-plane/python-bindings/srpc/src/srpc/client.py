# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from collections.abc import AsyncIterable
import datetime
import logging
from contextlib import asynccontextmanager

from google.rpc import code_pb2

import anyio
from srpc.codes import Code

import slim_bindings
from srpc.common import (
    create_local_app,
    service_and_method_to_pyname,
    split_id,
)

logging.basicConfig(level=logging.DEBUG)
logger = logging.getLogger(__name__)


class SRPCChannel:
    def __init__(
        self,
        local: str,
        slim: dict,
        enable_opentelemetry: bool = False,
        shared_secret: str | None = None,
    ):
        self.local = split_id(local)
        self.slim = slim
        self.enable_opentelemetry = enable_opentelemetry
        self.shared_secret = shared_secret

        self.handlers = {}

        self.local_app: slim_bindings.Slim = None

    async def connect(self, slim_service_name: str):
        # Create local SLIM instance
        self.local_app = await create_local_app(
            self.local,
            self.slim,
            enable_opentelemetry=self.enable_opentelemetry,
            shared_secret=self.shared_secret,
        )

        self.slim_service_name = split_id(slim_service_name)

        # Start receiving messages
        await self.local_app.__aenter__()

    def close(self) -> None:
        """
        Close the channel.
        """
        self.local_app.__aexit__(None, None, None)

    def stream_stream(
        self,
        method: str,
        request_serializer: callable = lambda x: x,
        response_deserializer: callable = lambda x: x,
    ):
        """
        Create a stream-stream callable for the given method.
        """

        async def call_stream_stream(
            requests,
            timeout=None,
            metadata=None,
            credentials=None,
            wait_for_ready=None,
            compression=None,
        ):
            service_name = service_and_method_to_pyname(self.slim_service_name, method)

            await self.local_app.set_route(
                service_name,
            )

            async with self.multi_callable_streams(
                service_name,
                request_serializer=request_serializer,
                response_deserializer=response_deserializer,
                stream_request=True,
                stream_response=True,
            ) as (recv_stream, send_stream):
                # Send stream of requests
                async for request in requests:
                    await send_stream.send(request)

                # Wait for 1 response
                return await recv_stream.receive()

        return call_stream_stream

    def stream_unary(
        self,
        method: str,
        request_serializer: callable = lambda x: x,
        response_deserializer: callable = lambda x: x,
    ):
        async def call_stream_unary(
            requests: AsyncIterable,
            timeout=None,
            metadata=None,
            credentials=None,
            wait_for_ready=None,
            compression=None,
        ):
            logger.info(f"----------------------------------------------")

            service_name = service_and_method_to_pyname(self.slim_service_name, method)

            await self.local_app.set_route(
                service_name,
            )

            async with self.multi_callable_streams(
                service_name,
                request_serializer=request_serializer,
                response_deserializer=response_deserializer,
                stream_request=True,
                stream_response=False,
            ) as (recv_stream, send_stream):
                logger.info(f"Sending stream-unary request to {service_name}")

                # Send stream of requests
                async for request in requests:
                    await send_stream.send(request)

                # Send a message to signal the stream is complete
                await send_stream.send()

                # Wait for 1 response
                return await recv_stream.receive()

        return call_stream_unary

    # def subscribe(
    #     self,
    #     callback: Callable[[ChannelConnectivity], None],
    #     try_to_connect: bool = False,
    # ) -> None: ...

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
            service_name = service_and_method_to_pyname(self.slim_service_name, method)

            await self.local_app.set_route(
                service_name,
            )

            # Create a session
            session = await self.local_app.create_session(
                slim_bindings.PySessionConfiguration.FireAndForget()
            )

            # Send the request
            request_bytes = request_serializer(request)
            await self.local_app.publish(
                session,
                request_bytes,
                dest=service_name,
                metadata=metadata,
            )

            # Wait for the responses
            async def generator():
                try:
                    while True:
                        session_recv, response_bytes = await self.local_app.receive(
                            session=session.id,
                        )

                        print(session_recv.metadata)
                        if session_recv.metadata.get("code") == str(Code.OK) and not request_bytes:
                            logger.info("End of stream received")
                            break

                        response = response_deserializer(response_bytes)
                        yield response
                except Exception as e:
                    logger.error(f"Error receiving messages: {e}")
                    raise

            async for response in generator():
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
            service_name = service_and_method_to_pyname(self.slim_service_name, method)

            await self.local_app.set_route(
                service_name,
            )

            # Create a session
            session = await self.local_app.create_session(
                slim_bindings.PySessionConfiguration.FireAndForget()
            )

            # Send the request
            request_bytes = request_serializer(request)
            await self.local_app.publish(
                session,
                request_bytes,
                dest=service_name,
                metadata=metadata,
            )

            # Wait for the response
            session_recv, response_bytes = await self.local_app.receive(
                session=session.id,
            )

            response = response_deserializer(response_bytes)

            return response

        return call_unary_unary

    # def unsubscribe(self, callback: Callable[[ChannelConnectivity], None]) -> None: ...
