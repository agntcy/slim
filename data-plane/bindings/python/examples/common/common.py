#!/usr/bin/env python3
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""Common utilities for SLIM Python binding examples."""

# Import from the packaged bindings
try:
    import slim_uniffi_bindings.generated.slim_bindings as slim
except ImportError:
    # Fallback to direct import if package not installed
    import sys
    import os
    GENERATED_DIR = os.path.join(os.path.dirname(__file__), '..', '..', 'slim_uniffi_bindings', 'generated')
    if os.path.exists(GENERATED_DIR):
        sys.path.insert(0, GENERATED_DIR)
    try:
        import slim_bindings as slim
    except ImportError:
        slim = None

def parse_name(name_str: str):
    """Parse a name string in format 'org/namespace/app' into a Name object.
    
    Args:
        name_str: Name string in format 'org/namespace/app'
        
    Returns:
        slim.Name object with components and id
    """
    if slim is None:
        raise ImportError("Could not import slim_bindings. Did you run 'task generate'?")
    
    components = name_str.split('/')
    if len(components) != 3:
        raise ValueError(f"Name must have 3 components separated by '/', got: {name_str}")
    
    return slim.Name(components=components, id=None)

def format_bytes(data: bytes) -> str:
    """Format bytes for display, showing printable text or hex for binary data.
    
    Args:
        data: Bytes to format
        
    Returns:
        Formatted string representation
    """
    try:
        return data.decode('utf-8')
    except UnicodeDecodeError:
        return data.hex()

def create_tls_config(insecure: bool = False, 
                     insecure_skip_verify: bool = False,
                     cert_file: str = None,
                     key_file: str = None,
                     ca_file: str = None):
    """Create a TLS configuration object.
    
    Args:
        insecure: Disable TLS entirely (plain text)
        insecure_skip_verify: Skip server certificate verification (testing only!)
        cert_file: Path to certificate file (PEM format)
        key_file: Path to private key file (PEM format)
        ca_file: Path to CA certificate file (PEM format)
        
    Returns:
        slim.TlsConfig object
    """
    if slim is None:
        raise ImportError("Could not import slim_bindings. Did you run 'task generate'?")
    
    return slim.TlsConfig(
        insecure=insecure,
        insecure_skip_verify=insecure_skip_verify,
        cert_file=cert_file,
        key_file=key_file,
        ca_file=ca_file,
        tls_version=None,
        include_system_ca_certs_pool=None
    )

def create_server_config(endpoint: str, tls_config = None):
    """Create a server configuration object.
    
    Args:
        endpoint: Server endpoint (e.g., "127.0.0.1:12345")
        tls_config: TLS configuration (default: insecure plain text)
        
    Returns:
        slim.ServerConfig object
    """
    if slim is None:
        raise ImportError("Could not import slim_bindings. Did you run 'task generate'?")
    
    if tls_config is None:
        tls_config = create_tls_config(insecure=True)
    
    return slim.ServerConfig(endpoint=endpoint, tls=tls_config)

def create_client_config(endpoint: str, tls_config = None):
    """Create a client configuration object.
    
    Args:
        endpoint: Server endpoint to connect to (e.g., "http://localhost:12345")
        tls_config: TLS configuration (default: insecure plain text)
        
    Returns:
        slim.ClientConfig object
    """
    if slim is None:
        raise ImportError("Could not import slim_bindings. Did you run 'task generate'?")
    
    if tls_config is None:
        tls_config = create_tls_config(insecure=True)
    
    return slim.ClientConfig(endpoint=endpoint, tls=tls_config)

def create_session_config(session_type: str = 'PointToPoint',
                         enable_mls: bool = False,
                         max_retries: int = None,
                         interval_ms: int = None,
                         initiator: bool = True,
                         metadata: dict = None):
    """Create a session configuration object.
    
    Args:
        session_type: 'PointToPoint' or 'Group'
        enable_mls: Enable MLS encryption
        max_retries: Maximum number of retries for message transmission
        interval_ms: Interval between retries in milliseconds
        initiator: Whether this endpoint is the session initiator
        metadata: Custom metadata key-value pairs
        
    Returns:
        slim.SessionConfig object
    """
    if slim is None:
        raise ImportError("Could not import slim_bindings. Did you run 'task generate'?")
    
    if metadata is None:
        metadata = {}
    
    # Convert string to SessionType enum
    if session_type == 'PointToPoint':
        session_type_enum = slim.SessionType.POINT_TO_POINT
    elif session_type == 'Group':
        session_type_enum = slim.SessionType.GROUP
    else:
        raise ValueError(f"Invalid session_type: {session_type}. Must be 'PointToPoint' or 'Group'")
    
    return slim.SessionConfig(
        session_type=session_type_enum,
        enable_mls=enable_mls,
        max_retries=max_retries,
        interval_ms=interval_ms,
        initiator=initiator,
        metadata=metadata
    )

