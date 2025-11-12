# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from datetime import timedelta

from slim_bindings._slim_bindings import (  # type: ignore[attr-defined]
    PyApp,
    PyIdentityProvider,
    PyIdentityVerifier,
    PyName,
    PySessionConfiguration,
    PySessionContext,
)

from .session import PySession


class Slim:
    """
    High-level façade over the underlying PyService (Rust core) providing a
    Pythonic API for:
      * Service initialization & authentication (via Slim.new)
      * Client connections to remote Slim services (connect / disconnect)
      * Server lifecycle management (run_server / stop_server)
      * Subscription & routing management (subscribe / unsubscribe / set_route / remove_route)
      * Session lifecycle (create_session / delete_session / listen_for_session)

    Core Concepts:
      - PyName: Fully-qualified name of the app (org / namespace / app-or-channel). Used for
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
      4. session = await slim.create_session(PySessionConfiguration.PointToPoint(peer_name=remote_name, ...))
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
        name: PyName,
        provider: PyIdentityProvider,
        verifier: PyIdentityVerifier,
        local_service: bool = False,
    ):
        """
        Primary constructor for initializing a Slim instance.

        Creates a Slim instance with the provided identity components and initializes
        the underlying service handle.

        Args:
            name (PyName): Fully qualified local name (org/namespace/app).
            provider (PyIdentityProvider): Identity provider for authentication.
            verifier (PyIdentityVerifier): Identity verifier for validating peers.
            local_service (bool): Whether this is a local service. Defaults to False.

        Note: Service initialization happens here via PyApp construction.
        """

        # Initialize service
        self._app = PyApp(
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
    def local_name(self) -> PyName:
        """Local fully-qualified PyName (org/namespace/app) for this app.

        Returns:
            PyName: Immutable name used for routing, subscriptions, etc.
        """
        return self._app.name

    def create_session(
        self,
        destination: PyName,
        session_config: PySessionConfiguration,
    ) -> PySession:
        """Create a new session and return its high-level PySession wrapper.

        Args:
            destination (PyName): Target peer or channel name.
            session_config (PySessionConfiguration): Parameters controlling creation.

        Returns:
            PySession: Wrapper exposing high-level async operations for the session.
        """
        ctx: PySessionContext = self._app.create_session(destination, session_config)
        return PySession(ctx)

    async def delete_session(self, session: PySession):
        """
        Terminate and remove an existing session.

        Args:
            session (PySession): Session wrapper previously returned by create_session.

        Returns:
            None

        Notes:
            Underlying errors from delete_session are propagated.
        """

        # Remove the session from SLIM
        await self._app.delete_session(session._ctx)

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
        name: PyName,
    ):
        """
        Add (or update) an explicit routing rule for outbound messages.

        Args:
            name (PyName): Destination app/channel name to route traffic toward.

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
        name: PyName,
    ):
        """
        Remove a previously established outbound routing rule.

        Args:
            name (PyName): Destination app/channel whose route should be removed.

        Returns:
            None
        """

        if self.conn_id is None:
            raise RuntimeError("No active connection. Please connect first.")

        await self._app.remove_route(
            name,
            self.conn_id,
        )

    async def subscribe(self, name: PyName):
        """
        Subscribe to inbound messages addressed to the specified name.

        Args:
            name (PyName): App or channel name to subscribe for deliveries.

        Returns:
            None
        """

        await self._app.subscribe(name, self.conn_id)

    async def unsubscribe(self, name: PyName):
        """
        Cancel a previous subscription for the specified name.

        Args:
            name (PyName): App or channel name whose subscription is removed.

        Returns:
            None
        """

        await self._app.unsubscribe(name, self.conn_id)

    async def listen_for_session(self, timeout: timedelta | None = None) -> PySession:
        """
        Await the next inbound session (optionally bounded by timeout).

        Returns:
            PySession: Wrapper for the accepted session context.
        """

        ctx: PySessionContext = await self._app.listen_for_session(timeout)
        return PySession(ctx)
