# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Shared helper utilities for the slim_bindings CLI examples.

This module centralizes:
  * Pretty-print / color formatting helpers
  * Identity (auth) helper constructors (shared secret)
  * Command-line option decoration (Click integration)
  * Convenience coroutine for constructing and connecting a local Slim app

The heavy inline commenting is intentional: it is meant to teach newcomers
exactly what each step does, line by line.
"""

import click  # CLI option parsing & command composition library.

# Import slim_uniffi_bindings
import slim_uniffi_bindings.generated.slim_bindings as slim


class color:
    """ANSI escape sequences for terminal styling."""

    PURPLE = "\033[95m"
    CYAN = "\033[96m"
    DARKCYAN = "\033[36m"
    BLUE = "\033[94m"
    GREEN = "\033[92m"
    YELLOW = "\033[93m"
    RED = "\033[91m"
    BOLD = "\033[1m"
    UNDERLINE = "\033[4m"
    END = "\033[0m"


def format_message(message1: str, message2: str = "") -> str:
    """
    Format a message for display with bold/cyan prefix column and optional suffix.

    Args:
        message1: Primary label (left column, capitalized & padded).
        message2: Optional trailing description/value.

    Returns:
        A colorized string ready to print.
    """
    return f"{color.BOLD}{color.CYAN}{message1.capitalize():<45}{color.END}{message2}"


def format_message_print(message1: str, message2: str = "") -> None:
    """Print a formatted message using format_message()."""
    print(format_message(message1, message2))


def split_id(id: str) -> slim.Name:
    """
    Split an ID of form organization/namespace/application (or channel).

    Args:
        id: String in the canonical 'org/namespace/app-or-stream' format.

    Raises:
        ValueError: If the id cannot be split into exactly three segments.

    Returns:
        Name: Constructed identity object.
    """
    try:
        components = id.split("/")
        if len(components) != 3:
            raise ValueError(f"ID must have 3 components, got {len(components)}")
    except Exception as e:
        print("Error: IDs must be in the format organization/namespace/app-or-stream.")
        raise e
    return slim.Name(components=components, id=None)


def create_tls_config(
    insecure: bool = False,
    insecure_skip_verify: bool = False,
    cert_file: str = None,
    key_file: str = None,
    ca_file: str = None,
) -> slim.TlsConfig:
    """
    Create a TLS configuration object.

    Args:
        insecure: Disable TLS entirely (plain text)
        insecure_skip_verify: Skip server certificate verification (testing only!)
        cert_file: Path to certificate file (PEM format)
        key_file: Path to private key file (PEM format)
        ca_file: Path to CA certificate file (PEM format)

    Returns:
        slim.TlsConfig object
    """
    return slim.TlsConfig(
        insecure=insecure,
        insecure_skip_verify=insecure_skip_verify,
        cert_file=cert_file,
        key_file=key_file,
        ca_file=ca_file,
        tls_version=None,
        include_system_ca_certs_pool=None,
    )


def create_client_config(endpoint: str, tls_config=None) -> slim.ClientConfig:
    """
    Create a client configuration object.

    Args:
        endpoint: Server endpoint (e.g., "http://127.0.0.1:12345")
        tls_config: TLS configuration (default: insecure plain text)

    Returns:
        slim.ClientConfig object
    """
    if tls_config is None:
        tls_config = create_tls_config(insecure=True)

    return slim.ClientConfig(endpoint=endpoint, tls=tls_config)


def create_server_config(endpoint: str, tls_config=None) -> slim.ServerConfig:
    """
    Create a server configuration object.

    Args:
        endpoint: Server endpoint (e.g., "127.0.0.1:12345")
        tls_config: TLS configuration (default: insecure plain text)

    Returns:
        slim.ServerConfig object
    """
    if tls_config is None:
        tls_config = create_tls_config(insecure=True)

    return slim.ServerConfig(endpoint=endpoint, tls=tls_config)


def create_session_config(
    session_type: str = "PointToPoint",
    enable_mls: bool = False,
    max_retries: int = None,
    interval_ms: int = None,
    initiator: bool = True,
    metadata: dict = None,
) -> slim.SessionConfig:
    """
    Create a session configuration object.

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
    if metadata is None:
        metadata = {}

    # Convert string to SessionType enum
    if session_type == "PointToPoint":
        session_type_enum = slim.SessionType.POINT_TO_POINT
    elif session_type == "Group":
        session_type_enum = slim.SessionType.GROUP
    else:
        raise ValueError(
            f"Invalid session_type: {session_type}. Must be 'PointToPoint' or 'Group'"
        )

    return slim.SessionConfig(
        session_type=session_type_enum,
        enable_mls=enable_mls,
        max_retries=max_retries,
        interval_ms=interval_ms,
        initiator=initiator,
        metadata=metadata,
    )


class DictParamType(click.ParamType):
    """Custom Click parameter type that interprets string input as JSON."""

    name = "dict"

    def convert(self, value, param, ctx):
        import json

        if isinstance(value, dict):
            return value
        try:
            return json.loads(value)
        except json.JSONDecodeError:
            self.fail(f"{value} is not valid JSON", param, ctx)


def common_options(function):
    """
    Decorator stacking all shared CLI options for example commands.
    """
    function = click.command(context_settings={"auto_envvar_prefix": "SLIM"})(function)

    function = click.option(
        "--local",
        type=str,
        required=True,
        help="Local ID in the format organization/namespace/application",
    )(function)

    function = click.option(
        "--remote",
        type=str,
        help="Remote ID in the format organization/namespace/application-or-stream",
    )(function)

    function = click.option(
        "--slim",
        default={
            "endpoint": "http://127.0.0.1:46357",
            "tls": {
                "insecure": True,
            },
        },
        type=DictParamType(),
        help="slim connection parameters",
    )(function)

    function = click.option(
        "--enable-opentelemetry",
        is_flag=True,
        help="Enable OpenTelemetry tracing",
    )(function)

    function = click.option(
        "--shared-secret",
        type=str,
        help="Shared secret for authentication. Don't use this in production.",
        default="abcde-12345-fedcb-67890-deadc",
    )(function)

    function = click.option(
        "--invites",
        type=str,
        multiple=True,
        help="Invite other participants to the group session. Can be specified multiple times.",
    )(function)

    function = click.option(
        "--enable-mls",
        is_flag=True,
        help="Enable MLS (Message Layer Security) for the session.",
    )(function)

    return function


async def create_local_app(
    local: str,
    slim_config: dict,
    remote: str | None = None,
    enable_opentelemetry: bool = False,
    shared_secret: str = "abcde-12345-fedcb-67890-deadc",
):
    """
    Build and connect a Slim application instance given user CLI parameters.

    Args:
        local: Local identity string (org/ns/app).
        slim_config: Dict of connection parameters (endpoint, tls flags, etc.).
        remote: Optional remote identity (unused here, reserved for future).
        enable_opentelemetry: Enable OTEL tracing export (note: tracing config not available in uniffi).
        shared_secret: Symmetric secret for shared-secret mode.

    Returns:
        tuple: (app, connection_id) - Connected Slim app instance and connection ID.
    """
    # Initialize crypto provider
    slim.initialize_crypto_provider()

    # Convert local identifier to a strongly typed Name.
    local_name = split_id(local)

    # Create app with shared secret
    app = slim.create_app_with_secret(local_name, shared_secret)

    # Provide feedback to user (instance numeric id).
    format_message_print(f"{app.id()}", "Created app")

    # Parse slim config and create client config
    endpoint = slim_config.get("endpoint", "http://127.0.0.1:46357")
    tls_insecure = slim_config.get("tls", {}).get("insecure", True)
    tls_skip_verify = slim_config.get("tls", {}).get("insecure_skip_verify", False)

    tls_config = create_tls_config(
        insecure=tls_insecure, insecure_skip_verify=tls_skip_verify
    )
    client_config = create_client_config(endpoint, tls_config)

    # Establish outbound connection using provided parameters (async version).
    conn_id = await app.connect_async(client_config)

    # Confirm endpoint connectivity.
    format_message_print(f"{app.id()}", f"Connected to {endpoint}")

    return app, conn_id
