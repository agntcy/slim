# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
from typing import Optional

from ._slim_bindings import (  # type: ignore[attr-defined]
    SESSION_UNSPECIFIED,
    PyAgentType,
    PyService,
    PySessionConfiguration,
    PySessionInfo,
    PySessionType,
    __version__,
    build_info,
    build_profile,
    connect,
    create_pyservice,
    create_session,
    delete_session,
    disconnect,
    get_default_session_config,
    get_session_config,
    publish,
    receive,
    remove_route,
    run_server,
    set_default_session_config,
    set_route,
    set_session_config,
    stop_server,
    subscribe,
    unsubscribe,
)
from ._slim_bindings import (
    PySessionDirection as PySessionDirection,
)
from ._slim_bindings import (
    init_tracing as init_tracing,
)


def get_version():
    """
    Get the version of the SLIM bindings.

    Returns:
        str: The version of the SLIM bindings.
    """
    return __version__


def get_build_profile():
    """
    Get the build profile of the SLIM bindings.

    Returns:
        str: The build profile of the SLIM bindings.
    """
    return build_profile


def get_build_info():
    """
    Get the build information of the SLIM bindings.

    Returns:
        str: The build information of the SLIM bindings.
    """
    return build_info


class SLIMTimeoutError(TimeoutError):
    """
    Exception raised for SLIM timeout errors.

    This exception is raised when an operation in an SLIM session times out.
    It encapsulates detailed information about the timeout event, including the
    ID of the message that caused the timeout and the session identifier. An
    optional underlying exception can also be provided to offer additional context.

    Attributes:
        message_id (int): The identifier associated with the message triggering the timeout.
        session_id (int): The identifier of the session where the timeout occurred.
        message (str): A brief description of the timeout error.
        original_exception (Exception, optional): The underlying exception that caused the timeout, if any.

    The string representation of the exception (via __str__) returns a full message that
    includes the custom message, session ID, and message ID, as well as details of the
    original exception (if present). This provides a richer context when the exception is logged
    or printed.
    """

    def __init__(
        self,
        message_id: int,
        session_id: int,
        message: str = "SLIM timeout error",
        original_exception: Optional[Exception] = None,
    ):
        self.message_id = message_id
        self.session_id = session_id
        self.message = message
        self.original_exception = original_exception
        full_message = f"{message} for session {session_id} and message {message_id}"
        if original_exception:
            full_message = f"{full_message}. Caused by: {original_exception!r}"
        super().__init__(full_message)

    def __str__(self):
        return self.args[0]

    def __repr__(self):
        return (
            f"{self.__class__.__name__}(session_id={self.session_id!r}, "
            f"message_id={self.message_id!r}, "
            f"message={self.message!r}, original_exception={self.original_exception!r})"
        )


class Slim:
    def __init__(
        self,
        svc: PyService,
        organization: str,
        namespace: str,
        agent: str,
    ):
        """
        Initialize a new SLIM instance. A SLIM instance is associated with a single
        local agent. The agent is identified by its organization, namespace, and name.
        The agent ID is determined by the provided service (svc).

        Args:
            svc (PyService): The Python service instance for SLIM.
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
        """

        # Initialize service
        self.svc = svc

        # Create sessions map
        self.sessions: dict[int, tuple[Optional[PySessionInfo], asyncio.Queue]] = {
            SESSION_UNSPECIFIED: (None, asyncio.Queue()),
        }

        # Save local names
        self.local_name = PyAgentType(organization, namespace, agent)
        self.local_id = self.svc.id

        # Create connection ID map
        self.conn_ids: dict[str, int] = {}

    async def __aenter__(self):
        """
        Start the receiver loop in the background.
        This function is called when the SLIM instance is used in a
        context manager (with statement).
        It will start the receiver loop in the background and return the
        SLIM instance.
        Args:
            None
        Returns:
            Slim: The SLIM instance.

        """

        # Run receiver loop in the background
        self.task = asyncio.create_task(self._receive_loop())
        return self

    async def __aexit__(self, exc_type, exc_value, traceback):
        """
        Stop the receiver loop.
        This function is called when the Slim instance is used in a
        context manager (with statement).
        It will stop the receiver loop and wait for it to finish.
        Args:
            exc_type: The exception type.
            exc_value: The exception value.
            traceback: The traceback object.
        Returns:
            None
        """

        # Cancel the receiver loop task
        self.task.cancel()

        # Wait for the task to finish
        try:
            await self.task
        except asyncio.CancelledError:
            pass

    @classmethod
    async def new(
        cls,
        organization: str,
        namespace: str,
        agent: str,
        agent_id: Optional[int] = None,
    ) -> "Slim":
        """
        Create a new SLIM instance. A SLIM instamce is associated to one single
        local agent. The agent is identified by its organization, namespace and name.
        The agent ID is optional. If not provided, the agent will be created with a new ID.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
            agent_id (int): The ID of the agent. If not provided, a new ID will be created.

        Returns:
            Slim: A new SLIM instance
        """

        return cls(
            await create_pyservice(organization, namespace, agent, agent_id),
            organization,
            namespace,
            agent,
        )

    def get_agent_id(self) -> int:
        """
        Get the ID of the agent.

        Args:
            None

        Returns:
            int: The ID of the agent.
        """

        return self.svc.id

    async def create_session(
        self,
        session_config: PySessionConfiguration,
        queue_size: int = 0,
    ) -> PySessionInfo:
        """
        Create a new streaming session.

        Args:
            session_config (PySessionConfiguration): The session configuration.
            queue_size (int): The size of the queue for the session.
                                If 0, the queue will be unbounded.
                                If a positive integer, the queue will be bounded to that size.

        Returns:
            ID of the session
        """

        session = await create_session(self.svc, session_config)
        self.sessions[session.id] = (session, asyncio.Queue(queue_size))
        return session

    async def delete_session(self, session_id: int):
        """
        Delete a session.

        Args:
            session_id (int): The ID of the session to delete.

        Returns:
            None

        Raises:
            ValueError: If the session ID is not found.
        """

        # Check if the session ID is in the sessions map
        if session_id not in self.sessions:
            raise ValueError(f"session not found: {session_id}")

        # Remove the session from the map
        del self.sessions[session_id]

        # Remove the session from SLIM
        await delete_session(self.svc, session_id)

    async def set_session_config(
        self,
        session_id: int,
        session_config: PySessionConfiguration,
    ):
        """
        Set the session configuration for a specific session.

        Args:
            session_id (int): The ID of the session.
            session_config (PySessionConfiguration): The new configuration for the session.

        Returns:
            None

        Raises:
            ValueError: If the session ID is not found.
        """

        # Check if the session ID is in the sessions map
        if session_id not in self.sessions:
            raise ValueError(f"session not found: {session_id}")

        # Set the session configuration
        await set_session_config(self.svc, session_id, session_config)

    async def get_session_config(
        self,
        session_id: int,
    ) -> PySessionConfiguration:
        """
        Get the session configuration for a specific session.

        Args:
            session_id (int): The ID of the session.

        Returns:
            PySessionConfiguration: The configuration of the session.

        Raises:
            ValueError: If the session ID is not found.
        """

        # Check if the session ID is in the sessions map
        if session_id not in self.sessions:
            raise ValueError(f"session not found: {session_id}")

        # Get the session configuration
        return await get_session_config(self.svc, session_id)

    async def set_default_session_config(
        self,
        session_config: PySessionConfiguration,
    ):
        """
        Set the default session configuration.

        Args:
            session_config (PySessionConfiguration): The new default session configuration.

        Returns:
            None
        """

        await set_default_session_config(self.svc, session_config)

    async def get_default_session_config(
        self,
        session_type: PySessionType,
    ) -> PySessionConfiguration:
        """
        Get the default session configuration.

        Args:
            session_id (int): The ID of the session.

        Returns:
            PySessionConfiguration: The default configuration of the session.
        """

        return await get_default_session_config(self.svc, session_type)

    async def run_server(self, config: dict):
        """
        Start the server part of the SLIM service. The server will be started only
        if its configuration is set. Otherwise, it will raise an error.

        Args:
            None

        Returns:
            None
        """

        await run_server(self.svc, config)

    async def stop_server(self, endpoint: str):
        """
        Stop the server part of the SLIM service.

        Args:
            None

        Returns:
            None
        """

        await stop_server(self.svc, endpoint)

    async def connect(self, client_config: dict) -> int:
        """
        Connect to a remote SLIM service.
        This function will block until the connection is established.

        Args:
            None

        Returns:
            int: The connection ID.
        """

        conn_id = await connect(
            self.svc,
            client_config,
        )

        # Save the connection ID
        self.conn_ids[client_config["endpoint"]] = conn_id

        # For the moment we manage one connection only
        self.conn_id = conn_id

        # Subscribe to the local name
        await subscribe(self.svc, conn_id, self.local_name, self.local_id)

        # return the connection ID
        return conn_id

    async def disconnect(self, endpoint: str):
        """
        Disconnect from a remote SLIM service.
        This function will block until the disconnection is complete.

        Args:
            None

        Returns:
            None

        """
        conn = self.conn_ids[endpoint]
        await disconnect(self.svc, conn)

    async def set_route(
        self,
        organization: str,
        namespace: str,
        agent: str,
        id: Optional[int] = None,
    ):
        """
        Set route for outgoing messages via the connected SLIM instance.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
            id (int): Optional ID of the agent.

        Returns:
            None
        """

        name = PyAgentType(organization, namespace, agent)
        await set_route(self.svc, self.conn_id, name, id)

    async def remove_route(
        self, organization: str, namespace: str, agent: str, id: Optional[int] = None
    ):
        """
        Remove route for outgoing messages via the connected SLIM instance.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
            id (int): Optional ID of the agent.

        Returns:
            None
        """

        name = PyAgentType(organization, namespace, agent)
        await remove_route(self.svc, self.conn_id, name, id)

    async def subscribe(
        self, organization: str, namespace: str, agent: str, id: Optional[int] = None
    ):
        """
        Subscribe to receive messages for the given agent.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
            id (int): Optional ID of the agent.

        Returns:
            None
        """

        sub = PyAgentType(organization, namespace, agent)
        await subscribe(self.svc, self.conn_id, sub, id)

    async def unsubscribe(
        self, organization: str, namespace: str, agent: str, id: Optional[int] = None
    ):
        """
        Unsubscribe from receiving messages for the given agent.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
            id (int): Optional ID of the agent.

        Returns:
            None
        """

        unsub = PyAgentType(organization, namespace, agent)
        await unsubscribe(self.svc, self.conn_id, unsub, id)

    async def publish(
        self,
        session: PySessionInfo,
        msg: bytes,
        organization: str,
        namespace: str,
        agent: str,
        agent_id: Optional[int] = None,
    ):
        """
        Publish a message to an agent via normal matching in subscription table.

        Args:
            session (PySessionInfo): The session information.
            msg (str): The message to publish.
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
            agent_id (int): Optional ID of the agent.

        Returns:
            None
        """

        # Make sure the sessions exists
        if session.id not in self.sessions:
            raise Exception("session not found", session.id)

        dest = PyAgentType(organization, namespace, agent)
        await publish(self.svc, session, 1, msg, dest, agent_id)

    async def request_reply(
        self,
        session: PySessionInfo,
        msg: bytes,
        organization: str,
        namespace: str,
        agent: str,
        agent_id: Optional[int] = None,
    ) -> tuple[PySessionInfo, Optional[bytes]]:
        """
        Publish a message and wait for the first response.

        Args:
            msg (str): The message to publish.
            session (PySessionInfo): The session information.
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.
            agent_id (int): Optional ID of the agent.

        Returns:
            tuple: The PySessionInfo and the message.
        """

        # Make sure the sessions exists
        if session.id not in self.sessions:
            raise Exception("Session ID not found")

        dest = PyAgentType(organization, namespace, agent)
        await publish(self.svc, session, 1, msg, dest, agent_id)

        # Wait for a reply in the corresponding session queue
        session_info, message = await self.receive(session.id)

        return session_info, message

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
        self, session: Optional[int] = None
    ) -> tuple[PySessionInfo, Optional[bytes]]:
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

    async def _receive_loop(self) -> None:
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
            except asyncio.CancelledError:
                raise
            except Exception as e:
                print("Error receiving message:", e)
                # Try to parse the error message
                try:
                    message_id, session_id, reason = parse_error_message(str(e))

                    # figure out what exception to raise based on the reason
                    if reason == "timeout":
                        err = SLIMTimeoutError(message_id, session_id)
                    else:
                        # we don't know the reason, just raise the original exception
                        raise e

                    if session_id in self.sessions:
                        await self.sessions[session_id][1].put(
                            err,
                        )
                    else:
                        print(self.sessions.keys())
                except Exception:
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
