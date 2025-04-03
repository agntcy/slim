# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
from typing import Optional, Tuple, Dict, Union

# import the contents of the Rust library into the Python extension
from ._agp_bindings import (
    PyGatewayConfig as GatewayConfig,
    PyService,
    PyAgentType,
    PySessionInfo,
    PyFireAndForgetConfiguration,
    PyRequestResponseConfiguration,
    create_ff_session,
    create_rr_session,
    create_pyservice,
    connect,
    disconnect,
    publish,
    receive,
    init_tracing,
    subscribe,
    unsubscribe,
    serve,
    stop,
    set_route,
    remove_route,
    SESSION_UNSPECIFIED,
)
from ._agp_bindings import __all__

# optional: include the documentation from the Rust module
from ._agp_bindings import __doc__  # noqa: F401


class TimeoutError(RuntimeError):
    """
    Custom exception class for timeout errors.
    """

    def __init__(self, message_id: int, session_id: int):
        self.message = f"Timeout error: message={message_id} session={session_id}"
        super().__init__(self.message)



class Gateway:
    def __init__(
        self,
        svc: PyService,
        id: Optional[int] = None,
    ):
        """
        Create a new Gateway instance. A gateway instamce is associated to one single
        local agent. The agent is identified by its organization, namespace and name.
        The agent ID is optional. If not provided, the agent will be created with a new ID.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
            id (int): The ID of the agent. If not provided, a new ID will be created.

        Returns:
            Gateway: A new Gateway instance
        """

        # Initialize service
        self.svc = svc

        # Run receiver loop in the background
        self.task = asyncio.create_task(self._receive_loop())

        # Create sessions map
        self.sessions: Dict[int, Tuple[PySessionInfo, asyncio.Queue]] = {
            SESSION_UNSPECIFIED: (None, asyncio.Queue()),
        }

        # Task for receiving messages
        self.task: Optional[asyncio.Task] = None

    @classmethod
    async def new(
        cls,
        organization: str,
        namespace: str,
        agent: str,
        id: Optional[int] = None,
    ) -> "Gateway":
        """
        Create a new Gateway instance. A gateway instamce is associated to one single
        local agent. The agent is identified by its organization, namespace and name.
        The agent ID is optional. If not provided, the agent will be created with a new ID.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
            id (int): The ID of the agent. If not provided, a new ID will be created.

        Returns:
            Gateway: A new Gateway instance
        """

        return cls(await create_pyservice(organization, namespace, agent, id))

    def id(self) -> int:
        """
        Get the ID of the agent.

        Args:
            None

        Returns:
            int: The ID of the agent.
        """

        return self.svc.id

    def configure(self, config: GatewayConfig):
        """
        Configure the gateway.

        Args:
            config (GatewayConfig): The gateway configuration class.

        Returns:
            None
        """

        self.svc.configure(config)

    async def create_ff_session(
        self,
        session_config: PyFireAndForgetConfiguration = PyFireAndForgetConfiguration(),
        queue_size: Optional[int] = 0,
    ) -> PySessionInfo:
        """
        Create a new session.

        Args:
            session_config (PyFireAndForgetConfiguration): The session configuration.
            queue_size (int): The size of the queue for the session.
                              If 0, the queue will be unbounded.
                              If a positive integer, the queue will be bounded to that size.

        Returns:
            ID of the session
        """

        session = await create_ff_session(self.svc, session_config)
        self.sessions[session.id] = (session, asyncio.Queue(queue_size))
        return session

    async def create_rr_session(
        self,
        session_config: PyRequestResponseConfiguration = PyRequestResponseConfiguration(),
        queue_size: Optional[int] = 0,
    ) -> PySessionInfo:
        """
        Create a new session.

        Args:
            session_config (PyRequestResponseConfiguration): The session configuration.
            queue_size (int): The size of the queue for the session.
                                If 0, the queue will be unbounded.
                                If a positive integer, the queue will be bounded to that size.

        Returns:
            ID of the session
        """

        session = await create_rr_session(self.svc, session_config)
        self.sessions[session.id] = (session, asyncio.Queue(queue_size))
        return session

    async def run_server(self):
        """
        Start the server part of the Gateway service. The server will be started only
        if its configuration is set. Otherwise, it will raise an error.

        Args:
            None

        Returns:
            None
        """

        await serve(self.svc)

    async def stop_server(self):
        """
        Stop the server part of the Gateway service.

        Args:
            None

        Returns:
            None
        """

        await stop(self.svc)

    async def connect(self) -> int:
        """
        Connect to a remote gateway service.
        This function will block until the connection is established.

        Args:
            None

        Returns:
            int: The connection ID.
        """

        self.conn_id = await connect(self.svc)

        return self.conn_id

    async def disconnect(self):
        """
        Disconnect from a remote gateway service.
        This function will block until the disconnection is complete.

        Args:
            None

        Returns:
            None

        """

        await disconnect(self.svc, self.conn_id)

    async def set_route(self, organization, namespace, agent, id: Optional[int] = None):
        """
        Set route for outgoing messages via the connected gateway.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
            id (int): The ID of the agent.

        Returns:
            None
        """

        name = PyAgentType(organization, namespace, agent)
        await set_route(self.svc, self.conn_id, name, id)

    async def remove_route(
        self, organization, namespace, agent, id: Optional[int] = None
    ):
        """
        Remove route for outgoing messages via the connected gateway.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.

        Returns:
            None
        """

        name = PyAgentType(organization, namespace, agent)
        await remove_route(self.svc, self.conn_id, name, id)

    async def subscribe(self, organization, namespace, agent, id=None):
        """
        Subscribe to receive messages for the given agent.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
            id (int): The ID of the agent.

        Returns:
            None
        """

        sub = PyAgentType(organization, namespace, agent)
        await subscribe(self.svc, self.conn_id, sub, id)

    async def unsubscribe(self, organization, namespace, agent, id=None):
        """
        Unsubscribe from receiving messages for the given agent.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
            id (int): The ID of the agent.

        Returns:
            None
        """

        unsub = PyAgentType(organization, namespace, agent)
        await unsubscribe(self.svc, self.conn_id, unsub, id)

    async def publish(self, session, msg, organization, namespace, agent):
        """
        Publish a message to an agent via normal matching in subscription table.

        Args:
            msg (str): The message to publish.
            session (PySessionInfo): The session information.
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.

        Returns:
            None
        """

        dest = PyAgentType(organization, namespace, agent)
        await publish(self.svc, session, 1, msg, dest, None)

    async def request_reply(
        self, session, msg, organization, namespace, agent
    ) -> Tuple[PySessionInfo, bytes]:
        """
        Publish a message and wait for the first response.

        Args:
            msg (str): The message to publish.
            session (PySessionInfo): The session information.
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.

        Returns:
            tuple: The PySessionInfo and the message.
        """

        # Make sure the sessions exists
        if session.id not in self.sessions:
            raise Exception("Session ID not found")

        dest = PyAgentType(organization, namespace, agent)
        await publish(self.svc, session, 1, msg, dest, None)

        # Wait for a reply in the corresponding session queue
        session_info, msg = await self.receive(session.id)

        return session_info, msg

    async def publish_to(self, session, msg):
        """
        Publish a message back to the agent that sent it.
        The information regarding the source agent is stored in the session.

        Args:
            session (PySessionInfo): The session information.
            msg (str): The message to publish.

        Returns:
            None
        """

        await publish(self.svc, session, 1, msg)

    async def receive(
        self, session: int = None
    ) -> Tuple[PySessionInfo, Optional[bytes]]:
        """
        Receive a message , optionally waiting for a specific session ID.
        If session ID is None, it will wait for new sessions to be created.
        This function will block until a message is received (if the session id is specified)
        or until a new session is created (if the session id is None).

        Args:
            session (int): The session ID. If None, the function will wait for any message.

        Returns:
            tuple: The PySessionInfo and the message.

        Raise:
            Exception: If the session ID is not found.
        """

        # If session is None, wait for any message
        if session is None:
            return await self.sessions[SESSION_UNSPECIFIED][1].get()
        else:
            # Check if the session ID is in the sessions map
            if session not in self.sessions:
                raise Exception("Session ID not found")

            # Get the queue for the session
            queue = self.sessions[session][1]

            # Wait for a message from the queue
            ret = await queue.get()

            # If message is am exception, raise it
            if isinstance(ret, Exception):
                raise ret

            # Otherwise, return the message
            return ret

    async def _receive_loop(self):
        """
        Receive messages in a loop running in the background.

        Returns:
            None
        """

        while True:
            try:
                session_info_msg = await receive(self.svc)

                id: int = session_info_msg[0].id

                # Check if the session ID is in the sessions map
                if id not in self.sessions:
                    # Create the entry in the sessions map
                    self.sessions[id] = (
                        session_info_msg,
                        asyncio.Queue(),
                    )

                    # Also add a queue for the session
                    await self.sessions[SESSION_UNSPECIFIED][1].put(session_info_msg)

                await self.sessions[id][1].put(session_info_msg)
            except Exception as e:
                print(e)

                # Try to parse the error message
                try:
                    message_id, session_id, reason = parse_error_message(str(e))

                    # figure out what exception to raise based on the reason
                    if reason == "timeout":
                        err = TimeoutError(message_id, session_id)
                    else:
                        # we don't know the reason, just raise the original exception
                        raise e

                    if session_id in self.sessions:
                        await self.sessions[session_id][1].put(
                            err,
                        )
                    else:
                        print(self.sessions.keys())
                except:
                    raise e


def parse_error_message(error_message):
    import re

    # Define the regular expression pattern
    pattern = r"message=(\d+) session=(\d+): (.+)"

    # Use re.search to find the pattern in the string
    match = re.search(pattern, error_message)

    if match:
        # Extract message_id, session_id, and reason from the match groups
        message_id = match.group(1)
        session_id = match.group(2)
        reason = match.group(3)
        return int(message_id), int(session_id), reason
    else:
        raise ValueError("error message does not match the expected format.")
