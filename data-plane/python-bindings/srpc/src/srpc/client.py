# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import logging
from collections.abc import AsyncIterable

from google.rpc import code_pb2, status_pb2

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
        async def call_stream_stream(
            request_stream: AsyncIterable,
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

            # TODO: check how this is done in grpc

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

                        print(session_recv.metadata)
                        if (
                            session_recv.metadata.get("code") == str(code_pb2.OK)
                            and not response_bytes
                        ):
                            logger.info("End of stream received")
                            break

                        response = response_deserializer(response_bytes)
                        yield response
                except Exception as e:
                    logger.error(f"Error receiving messages: {e}")
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
            service_name = service_and_method_to_pyname(self.slim_service_name, method)

            await self.local_app.set_route(
                service_name,
            )

            # Create a session
            session = await self.local_app.create_session(
                slim_bindings.PySessionConfiguration.FireAndForget()
            )

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

            # Wait for response
            session_recv, response_bytes = await self.local_app.receive(
                session=session.id,
            )

            response = response_deserializer(response_bytes)

            return response

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
                        if (
                            session_recv.metadata.get("code") == str(code_pb2.OK)
                            and not response_bytes
                        ):
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
