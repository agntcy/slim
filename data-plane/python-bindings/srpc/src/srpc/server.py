# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging

import slim_bindings
from srpc.common import (
    create_local_app,
    split_id,
)
from srpc.context import Context
from srpc.rpc import Rpc

logging.basicConfig(level=logging.DEBUG)
logger = logging.getLogger(__name__)


class Server:
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

    async def register_method_handlers(
        self, service_name: str, handlers: dict[str, Rpc]
    ):
        """
        Register method handlers for the server.
        """
        for method_name, handler in handlers.items():
            handler.method_name = method_name
            handler.service_name = service_name

            self.register_rpc(handler)

    def register_rpc(self, rpc_handler: Rpc):
        """
        Register an RPC handler for the server.
        """

        # Compose a PyName using the fist components of the local name and the RPC name
        subscription_name = self.handler_name_to_pyname(rpc_handler)

        # Register the RPC handler
        self.handlers[subscription_name] = rpc_handler

    async def run(self):
        # Create local SLIM instance
        local_app = await create_local_app(
            self.local,
            self.slim,
            enable_opentelemetry=self.enable_opentelemetry,
            shared_secret=self.shared_secret,
        )

        # Subscribe
        for s, h in self.handlers.items():
            await local_app.subscribe(s)

        instance = local_app.get_id()

        async with local_app:
            # Wait for a message and reply in a loop
            while True:
                logging.info(f"{instance} waiting for new session to be established")

                session_info, _ = await local_app.receive()
                logging.info(
                    f"{instance} received a new session: {session_info.id}",
                )

                asyncio.create_task(self.handle_session(session_info, local_app))

    async def handle_session(self, session_info, local_app: slim_bindings.Slim):
        session_id = session_info.id
        instance = local_app.get_id()

        while True:
            # Receive the message from the session
            session, msg = await local_app.receive(session=session_id)
            logging.info(
                f"{instance} received (from session {session_id}): {msg.decode()}",
            )

            # build the context for the RPC call
            context = Context.from_sessioninfo(session)

            # Call the RPC handler
            if session_id.destination_name in self.handlers:
                rpc_handler: Rpc = self.handlers[session_id.destination_name]
                request = rpc_handler.request_deserializer(msg)

                logging.info(
                    f"{instance} calling handler {rpc_handler.name} for session {session_id.id}",
                )

                # Send the response back to the client
                if rpc_handler.response_streaming:
                    async for code, response in rpc_handler.call_handler(
                        request, context
                    ):
                        response_bytes = rpc_handler.response_serializer(response)
                        await local_app.publish_to(
                            session_info, response_bytes, metadata={"code": code}
                        )

                    return

                response = await rpc_handler.call_handler(request, context)
                response_bytes = rpc_handler.response_serializer(response)
                await local_app.publish_to(
                    session_info,
                    response_bytes,
                    metadata={"code": 200},
                )

                # TODO(msardara): handle cleanup

    def handler_name_to_pyname(self, rpc_handler: Rpc) -> slim_bindings.PyName:
        """
        Convert a handler name to a PyName.
        """

        components = self.local.components_strings()

        subscription_name = slim_bindings.PyName(
            components[0],
            components[1],
            f"{components[2]}-{rpc_handler.service_name}-{rpc_handler.method_name}",
        )

        return subscription_name

    def pyname_to_handler_name(self, subscription_name: slim_bindings.PyName) -> str:
        """
        Convert a PyName to a handler name.
        """

        return subscription_name.components[3]
