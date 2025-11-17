# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import typing
from datetime import timedelta

from slim_bindings._slim_bindings import (
    App,
    IdentityProvider,
    IdentityVerifier,
    Name,
    SessionConfiguration,
    SessionContext,
)

from .session import Session


class Slim:
    """
    High-level façade over the underlying Service (Rust core) providing a
    Pythonic API for:
      * Service initialization & authentication (via Slim.new)
      * Client connections to remote Slim services (connect / disconnect)
      * Server lifecycle management (run_server / stop_server)
      * Subscription & routing management (subscribe / unsubscribe / set_route / remove_route)
      * Session lifecycle (create_session / delete_session / listen_for_session)

    Core Concepts:
      - Name: Fully-qualified name of the app (org / namespace / app-or-channel). Used for
        routing, subscriptions.
      - Session: Logical communication context. Types supported include:
          * PointToPoint  : Point-to-point with a fixed, stable destination (sticky).
          * Group: Many-to-many via a named channel/topic.
      - Default Session Configuration: A fallback used when inbound sessions are created
        towards this service (set via set_default_session_config).

    Typical Lifecycle (Client):
      1. slim = await Slim.new(local_name, identity_provider, identity_verifier)
      2. await slim.connect({"endpoint": "...", "tls": {"insecure": True}})
      3. await slim.set_route(remote_name)
      4. session = await slim.create_session(SessionConfiguration.PointToPoint(peer_name=remote_name, ...))
      5. await session.publish(b"payload")
      6. await slim.delete_session(session)
      7. await slim.disconnect("endpoint-string")

    Typical Lifecycle (Server):
      1. slim = await Slim.new(local_name, provider, verifier)
      2. await slim.run_server({"endpoint": "127.0.0.1:12345", "tls": {"insecure": True}})
      3. inbound = await slim.listen_for_session()
      4. msg_ctx, data = await inbound.get_message()
      5. await inbound.publish_to(msg_ctx, b"reply")
      6. await slim.stop_server("127.0.0.1:12345")

    Threading / Concurrency:
      - All network / I/O operations are async and awaitable.
      - A single Slim instance can service multiple concurrent awaiters.

    Error Handling:
      - Methods propagate underlying exceptions (e.g., invalid routing, closed sessions).
      - connect / run_server may raise if the endpoint is unreachable or already bound.

    Performance Notes:
      - Route changes are lightweight but may take a short time to propagate remotely.
      - listen_for_session can be long-lived; provide a timeout if you need bounded wait.

    Security Notes:
      - Identity provider & verifier determine trust model (e.g. shared secret vs JWT).
      - For production, prefer asymmetric keys / JWT over shared secrets.

    """

    def __init__(
        self,
        name: Name,
        provider: IdentityProvider,
        verifier: IdentityVerifier,
        local_service: bool = False,
    ):
        """
        Primary constructor for initializing a Slim instance.

        Creates a Slim instance with the provided identity components and initializes
        the underlying service handle.

        Args:
            name (Name): Fully qualified local name (org/namespace/app).
            provider (IdentityProvider): Identity provider for authentication.
            verifier (IdentityVerifier): Identity verifier for validating peers.
            local_service (bool): Whether this is a local service. Defaults to False.

        Note: Service initialization happens here via PyApp construction.
        """

        # Initialize service
        self._app = App(
            name,
            provider,
            verifier,
            local_service=local_service,
        )

        # Create connection ID map
        self.conn_ids: dict[str, int] = {}

        # For the moment we manage one connection only
        self.conn_id: int | None = None

    @property
    def id(self) -> int:
        """Unique numeric identifier of the underlying app instance.

        Returns:
            int: Service ID allocated by the native layer.
        """
        return self._app.id

    @property
    def id_str(self) -> str:
        """String representation of the unique identifier of the underlying app instance.

        Returns:
            str: String representation of the Service ID allocated by the native layer.
        """

        components_string = self.local_name.components_strings()

        return f"{components_string[0]}/{components_string[1]}/{components_string[2]}/{self._app.id}"

    @property
    def local_name(self) -> Name:
        """Local fully-qualified Name (org/namespace/app) for this app.

        Returns:
            Name: Immutable name used for routing, subscriptions, etc.
        """
        return self._app.name

    async def create_session(
        self,
        destination: Name,
        session_config: SessionConfiguration,
    ) -> typing.Any:
        """Create a new session and return its high-level Session wrapper.

        Args:
            destination (Name): Target peer or channel name.
            session_config (SessionConfiguration): Parameters controlling creation.

        Returns:
            Session: Wrapper exposing high-level async operations for the session.
        """
        ctx, completion_handle = await self._app.create_session(
            destination, session_config
        )
        return Session(ctx), completion_handle

    async def delete_session(self, session: Session) -> typing.Any:
        """
        Terminate and remove an existing session.

        Args:
            session (Session): Session wrapper previously returned by create_session.

        Returns:
            None

        Notes:
            Underlying errors from delete_session are propagated.
        """

        # Remove the session from SLIM
        return await self._app.delete_session(session._ctx)

    async def run_server(self, config: dict):
        """
        Start a GRPC server component with the supplied config.
        This allocates network resources (e.g. binds listening sockets).

        Args:
            config (dict): Server configuration parameters (check SLIM configuration for examples).

        Returns:
            None
        """

        await self._app.run_server(config)

    async def stop_server(self, endpoint: str):
        """
        Stop the server component listening at the specified endpoint.

        Args:
            endpoint (str): Endpoint identifier / address previously passed to run_server.

        Returns:
            None
        """

        await self._app.stop_server(endpoint)

    async def connect(self, client_config: dict) -> int:
        """
        Establish an outbound connection to a remote SLIM service.
        Awaits completion until the connection is fully established and subscribed.

        Args:
            client_config (dict): Dial parameters; must include 'endpoint'.

        Returns:
            int: Numeric connection identifier assigned by the service.
        """

        conn_id = await self._app.connect(client_config)

        # Save the connection ID
        self.conn_ids[client_config["endpoint"]] = conn_id

        # For the moment we manage one connection only
        self.conn_id = conn_id

        # Subscribe to the local name
        await self._app.subscribe(
            self._app.name,
            conn_id,
        )

        # return the connection ID
        return conn_id

    async def disconnect(self, endpoint: str):
        """
        Disconnect from a previously established remote connection.
        Awaits completion; underlying resources are released before return.

        Args:
            endpoint (str): The endpoint string used when connect() was invoked.

        Returns:
            None

        """
        conn = self.conn_ids[endpoint]
        await self._app.disconnect(conn)

    async def set_route(
        self,
        name: Name,
    ):
        """
        Add (or update) an explicit routing rule for outbound messages.

        Args:
            name (Name): Destination app/channel name to route traffic toward.

        Returns:
            None
        """

        if self.conn_id is None:
            raise RuntimeError("No active connection. Please connect first.")

        await self._app.set_route(
            name,
            self.conn_id,
        )

    async def remove_route(
        self,
        name: Name,
    ):
        """
        Remove a previously established outbound routing rule.

        Args:
            name (Name): Destination app/channel whose route should be removed.

        Returns:
            None
        """

        if self.conn_id is None:
            raise RuntimeError("No active connection. Please connect first.")

        await self._app.remove_route(
            name,
            self.conn_id,
        )

    async def subscribe(self, name: Name):
        """
        Subscribe to inbound messages addressed to the specified name.

        Args:
            name (Name): App or channel name to subscribe for deliveries.

        Returns:
            None
        """

        await self._app.subscribe(name, self.conn_id)

    async def unsubscribe(self, name: Name):
        """
        Cancel a previous subscription for the specified name.

        Args:
            name (Name): App or channel name whose subscription is removed.

        Returns:
            None
        """

        await self._app.unsubscribe(name, self.conn_id)

    async def listen_for_session(self, timeout: timedelta | None = None) -> Session:
        """
        Await the next inbound session (optionally bounded by timeout).

        Returns:
            Session: Wrapper for the accepted session context.
        """

        ctx: SessionContext = await self._app.listen_for_session(timeout)
        return Session(ctx)
