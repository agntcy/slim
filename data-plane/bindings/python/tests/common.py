# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import slim_uniffi_bindings._slim_bindings.slim_bindings as slim_bindings


def create_name(org: str, namespace: str, app: str, id: int = None) -> slim_bindings.Name:
    """Create a Name with the new API structure.
    
    Args:
        org: Organization component
        namespace: Namespace component  
        app: Application component
        id: Optional numeric ID
        
    Returns:
        slim_bindings.Name: Name object with components list
    """
    return slim_bindings.Name(components=[org, namespace, app], id=id)


def create_client_config(endpoint: str, insecure: bool = True) -> slim_bindings.ClientConfig:
    """Create a ClientConfig for connecting to a SLIM server.
    
    Args:
        endpoint: Server endpoint (e.g., "http://127.0.0.1:12345")
        insecure: Whether to use insecure TLS (default: True for tests)
        
    Returns:
        slim_bindings.ClientConfig: Client configuration object
    """
    return slim_bindings.ClientConfig(
        endpoint=endpoint,
        tls=slim_bindings.TlsConfig(
            insecure=insecure,
            insecure_skip_verify=False,
            cert_file=None,
            key_file=None,
            ca_file=None,
            tls_version=None,
            include_system_ca_certs_pool=None,
        )
    )


def create_server_config(endpoint: str, insecure: bool = True) -> slim_bindings.ServerConfig:
    """Create a ServerConfig for running a SLIM server.
    
    Args:
        endpoint: Server bind address (e.g., "127.0.0.1:12345")
        insecure: Whether to use insecure TLS (default: True for tests)
        
    Returns:
        slim_bindings.ServerConfig: Server configuration object
    """
    return slim_bindings.ServerConfig(
        endpoint=endpoint,
        tls=slim_bindings.TlsConfig(
            insecure=insecure,
            insecure_skip_verify=False,
            cert_file=None,
            key_file=None,
            ca_file=None,
            tls_version=None,
            include_system_ca_certs_pool=None,
        )
    )


def create_svc(
    name: slim_bindings.Name,
    secret: str = "testing-secret-123456789012345abc",
    local_service: bool = True,
):
    """Create and return a BindingsAdapter (low-level app) for tests using SharedSecret auth.

    Args:
        name: Fully qualified Name identifying the local service/app.
        secret: Shared secret string used for symmetric token generation/verification.
        local_service: Whether to use a local service instance or the global one.

    Returns:
        BindingsAdapter: The app instance usable with session creation and message operations.
    """
    # The new API only exposes create_app_with_secret() which handles auth internally
    # We cannot use local_service parameter with this function, so we ignore it for now
    return slim_bindings.create_app_with_secret(name, secret)


def create_slim(
    name: slim_bindings.Name,
    secret: str = "testing-secret-123456789012345abc",
    local_service: bool = True,
):
    """Create and return a BindingsAdapter (app) instance for tests using SharedSecret auth.

    This is an alias for create_svc() to maintain compatibility with tests that use
    the old "Slim" naming. Both return the same BindingsAdapter type.

    Args:
        name: Fully qualified Name for the local application/service.
        secret: Shared secret used for symmetric identity provider/verifier.
        local_service: Whether to use a local service instance (ignored in new API).

    Returns:
        BindingsAdapter: App instance with access to session and message operations.
    """
    return create_svc(name, secret, local_service)
