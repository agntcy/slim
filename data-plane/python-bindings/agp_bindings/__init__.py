# SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
# SPDX-License-Identifier: Apache-2.0

# import the contents of the Rust library into the Python extension
from typing import Optional, Tuple
from ._agp_bindings import (
    PyService,
    PyAgentClass,
    create_agent,
    connect,
    disconnect,
    publish,
    receive,
    init_tracing,
    subscribe,
    unsubscribe,
    serve,
    set_route,
    remove_route,
)
from ._agp_bindings import __all__

# optional: include the documentation from the Rust module
from ._agp_bindings import __doc__  # noqa: F401


class Gateway:
    def __init__(self, name="gateway/agent"):
        """
        Create a new Gateway instance.

        Args:
            name (str): The name of the Gateway service. Default is "gateway/agent".

        Returns:
            Gateway: A new Gateway instance
        """
        self.svc = PyService(name)

    async def create_agent(
        self, organization, namespace, agent, id: Optional[int] = None
    ) -> int:
        """
        Create a new agent.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.

        Returns:
            None
        """

        return await create_agent(self.svc, organization, namespace, agent, id)

    async def serve(
        self,
        address,
        insecure: Optional[bool] = False,
        tls_server_cert_path: Optional[str] = None,
        tls_server_key_path: Optional[str] = None,
        tls_server_cert_pem: Optional[str] = None,
        tls_server_key_pem: Optional[str] = None,
        basic_auth_username: Optional[str] = None,
        basic_auth_password: Optional[str] = None,
    ):
        """
        Serve the Gateway service.

        Args:
            address (str): The address to serve the Gateway service on.
            insecure (bool): Disable TLS. Default is False.
            tls_server_cert_path (str): Path to the server certificate.
            tls_server_key_path (str): Path to the server key.
            tls_server_cert_pem (str): Server certificate as PEM.
            tls_server_key_pem (str): Server key as PEM.
            basic_auth_username (str): Username for basic auth.
            basic_auth_password (str): Password for basic auth.

        Returns:
            None
        """

        await agp_bindings.serve(
            self.svc,
            address,
            insecure,
            tls_server_cert_path,
            tls_server_key_path,
            tls_server_cert_pem,
            tls_server_key_pem,
            basic_auth_username,
            basic_auth_password,
        )

    async def connect(
        self,
        endpoint,
        insecure: Optional[bool] = False,
        insecure_skip_verify: Optional[bool] = False,
        tls_ca_path: Optional[str] = None,
        tls_client_cert_path: Optional[str] = None,
        tls_client_key_path: Optional[str] = None,
        tls_ca_pem: Optional[str] = None,
        tls_client_cert_pem: Optional[str] = None,
        tls_client_key_pem: Optional[str] = None,
        basic_auth_username: Optional[str] = None,
        basic_auth_password: Optional[str] = None,
    ) -> int:
        """
        Connect to a remote gateway service.

        Args:
            endpoint (str): The endpoint of the remote gateway service.
            insecure (bool): Disable TLS. Default is False.
            insecure_skip_verify (bool): Skip TLS verification. Default is False.
            tls_ca_path (str): Path to the CA certificate.
            tls_client_cert_path (str): Path to the client certificate.
            tls_client_key_path (str): Path to the client key.
            tls_ca_pem (str): CA certificate as PEM.
            tls_client_cert_pem (str): Client certificate as PEM.
            tls_client_key_pem (str): Client key as PEM.
            basic_auth_username (str): Username for basic auth.
            basic_auth_password (str): Password for basic auth.

        Returns:
            int: The connection ID.
        """

        self.conn_id = await connect(
            self.svc,
            endpoint,
            insecure,
            insecure_skip_verify,
            tls_ca_path,
            tls_client_cert_path,
            tls_client_key_path,
            tls_ca_pem,
            tls_client_cert_pem,
            tls_client_key_pem,
            basic_auth_username,
            basic_auth_password,
        )

        return self.conn_id
    
    async def disconnect(self):
        """
        disconnect from a remote gateway service.

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

        Returns:
            None
        """

        name = PyAgentClass(organization, namespace, agent)
        await set_route(self.svc, self.conn_id, name, id)

    async def remove_route(self, organization, namespace, agent, id: Optional[int] = None):
        """
        Remove route for outgoing messages via the connected gateway.

        Args:
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.

        Returns:
            None
        """

        name = PyAgentClass(organization, namespace, agent)
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

        sub = PyAgentClass(organization, namespace, agent)
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
        unsub = PyAgentClass(organization, namespace, agent)
        await unsubscribe(self.svc, self.conn_id, unsub, id)

    async def publish(self, msg, organization, namespace, agent):
        """
        Publish a message to an agent via normal matching in subscription table.

        Args:
            msg (str): The message to publish.
            organization (str): The organization of the agent.
            namespace (str): The namespace of the agent.
            agent (str): The name of the agent.

        Returns:
            None
        """

        dest = PyAgentClass(organization, namespace, agent)
        await publish(self.svc, 1, msg, dest, None)

    async def publish_to(self, msg, agent):
        """
        Publish a message to an agent via the connected gateway.

        Args:
            msg (str): The message to publish.
            agent (Agent): The agent to publish to.

        Returns:
            None
        """

        await publish(self.svc, 1, msg, agent=agent)

    async def receive(self) -> Tuple[any, bytes]:
        """
        Receive a message from the connected gateway.

        Returns:
            tuple: The source agent and the message.

        """

        source, msg = await receive(self.svc)
        return source, msg
