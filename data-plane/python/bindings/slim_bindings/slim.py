# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
from datetime import timedelta
from typing import Optional

from slim_bindings._slim_bindings import (  # type: ignore[attr-defined]
    PyIdentityProvider,
    PyIdentityVerifier,
    PyName,
    PyService,
    PySessionConfiguration,
    PySessionContext,
    connect,
    create_pyservice,
    create_session,
    delete_session,
    disconnect,
    listen_for_session,
    remove_route,
    run_server,
    set_default_session_config,
    set_route,
    stop_server,
    subscribe,
    unsubscribe,
)

from .session import PySession


class Slim:
    def __init__(
        self,
        svc: PyService,
        name: PyName,
    ):
        """
        Initialize a new SLIM instance. A SLIM instance is associated with a single
        local app. The app is identified by its organization, namespace, and name.
        The unique ID is determined by the provided service (svc).

        Args:
            svc (PyService): The Python service instance for SLIM.
            organization (str): The organization of the app.
            namespace (str): The namespace of the app.
            app (str): The name of the app.
        """

        # Initialize service
        self._svc = svc

        # Save local names
        name.id = svc.id
        self.local_name = name

        # Create connection ID map
        self.conn_ids: dict[str, int] = {}

    @classmethod
    async def new(
        cls,
        name: PyName,
        provider: PyIdentityProvider,
        verifier: PyIdentityVerifier,
    ) -> "Slim":
        """
        Create a new SLIM instance. A SLIM instance is associated to one single
        local app. The app is identified by its organization, namespace and name.
        The app ID is optional. If not provided, the app will be created with a new ID.

        Args:
            organization (str): The organization of the app.
            namespace (str): The namespace of the app.
            app (str): The name of the app.
            app_id (int): The ID of the app. If not provided, a new ID will be created.

        Returns:
            Slim: A new SLIM instance
        """

        return cls(
            await create_pyservice(name, provider, verifier),
            name,
        )

    def get_id(self) -> int:
        """
        Get the ID of the app.

        Args:
            None

        Returns:
            int: The ID of the app.
        """

        return self._svc.id

    async def create_session(
        self,
        session_config: PySessionConfiguration,
    ) -> PySession:
        """Create and return a high-level `Session` wrapper.

        Args:
            session_config: The configuration for the new session.

        Returns:
            Session: Python wrapper around the created session context.
        """
        ctx: PySessionContext = await create_session(self._svc, session_config)
        return PySession(self._svc, ctx)

    async def delete_session(self, session: PySession):
        """
        Delete a session.

        Args:
            session_ctx (PySessionContext): The context of the session to delete.

        Returns:
            None

        Raises:
            ValueError: If the session ID is not found.
        """

        # Remove the session from SLIM
        await delete_session(self._svc, session._ctx)

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

        await set_default_session_config(self._svc, session_config)

    async def run_server(self, config: dict):
        """
        Start the server part of the SLIM service. The server will be started only
        if its configuration is set. Otherwise, it will raise an error.

        Args:
            None

        Returns:
            None
        """

        await run_server(self._svc, config)

    async def stop_server(self, endpoint: str):
        """
        Stop the server part of the SLIM service.

        Args:
            None

        Returns:
            None
        """

        await stop_server(self._svc, endpoint)

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
            self._svc,
            client_config,
        )

        # Save the connection ID
        self.conn_ids[client_config["endpoint"]] = conn_id

        # For the moment we manage one connection only
        self.conn_id = conn_id

        # Subscribe to the local name
        await subscribe(self._svc, conn_id, self.local_name)

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
        await disconnect(self._svc, conn)

    async def set_route(
        self,
        name: PyName,
    ):
        """
        Set route for outgoing messages via the connected SLIM instance.

        Args:
            name (PyName): The name of the app or channel to route messages to.

        Returns:
            None
        """

        await set_route(self._svc, self.conn_id, name)

    async def remove_route(
        self,
        name: PyName,
    ):
        """
        Remove route for outgoing messages via the connected SLIM instance.

        Args:
            name (PyName): The name of the app or channel to remove the route for.

        Returns:
            None
        """

        await remove_route(self._svc, self.conn_id, name)

    async def subscribe(self, name: PyName):
        """
        Subscribe to receive messages for the given name.

        Args:
            name (PyName): The name to subscribe to. This can be an app or a channel.

        Returns:
            None
        """

        await subscribe(self._svc, self.conn_id, name)

    async def unsubscribe(self, name: PyName):
        """
        Unsubscribe from receiving messages for the given name.

        Args:
            name (PyName): The name to unsubscribe from. This can be an app or a channel.

        Returns:
            None
        """

        await unsubscribe(self._svc, self.conn_id, name)

    async def listen_for_session(
        self, timeout: Optional[timedelta] = None
    ) -> PySession:
        """
        Wait for a new session to be established.

        Returns:
            PySessionContext: the new session
        """

        if timeout is None:
            # Use a very large timeout value instead of trying to use datetime.max
            timeout = timedelta(days=365 * 100)  # ~100 years

        async with asyncio.timeout(timeout.total_seconds()):
            session_ctx = await listen_for_session(self._svc)
            return PySession(self._svc, session_ctx)
