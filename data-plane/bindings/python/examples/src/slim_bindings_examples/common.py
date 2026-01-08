# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Shared helper utilities for the slim_bindings CLI examples.

This module centralizes:
  * Pretty-print / color formatting helpers
  * Identity (auth) helper constructors (shared secret / JWT / JWKS / SPIRE)
  * Command-line option decoration (Click integration)
  * Convenience coroutine for constructing and connecting a local Slim app

The heavy inline commenting is intentional: it is meant to teach newcomers
exactly what each step does, line by line.
"""

import base64  # Used to decode base64-encoded JWKS content (when provided).
import json  # Used for parsing JWKS JSON and dynamic option values.

import click  # CLI option parsing & command composition library.

# Import slim_uniffi_bindings (Maturin-generated UniFFI module)
import slim_uniffi_bindings._slim_bindings.slim_bindings as slim


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


def jwt_identity(
    jwt_path: str,
    spire_bundle_path: str,
    iss: str | None = None,
    sub: str | None = None,
    aud: list[str] | None = None,
):
    """
    Construct a static-JWT provider and JWT verifier from file inputs.

    Process:
      1. Read a JSON structure containing (base64-encoded) JWKS data (a SPIRE
         bundle with a JWKS for each trust domain).
      2. Decode & merge all JWKS entries.
      3. Create a StaticJwt identity provider pointing at a local JWT file.
      4. Wrap merged JWKS JSON as Key with RS256 & JWKS format.
      5. Build a Jwt verifier using the JWKS-derived public key.
    """
    print(f"Using SPIRE bundle file: {spire_bundle_path}")

    with open(spire_bundle_path) as sb:
        spire_bundle_string = sb.read()

    spire_bundle = json.loads(spire_bundle_string)

    all_keys = []
    for trust_domain, v in spire_bundle.items():
        print(f"Processing trust domain: {trust_domain}")
        try:
            decoded_jwks = base64.b64decode(v)
            jwks_json = json.loads(decoded_jwks)
            if "keys" in jwks_json:
                all_keys.extend(jwks_json["keys"])
                print(f"  Added {len(jwks_json['keys'])} keys from {trust_domain}")
            else:
                print(f"  Warning: No 'keys' found in JWKS for {trust_domain}")
        except (json.JSONDecodeError, UnicodeDecodeError, ValueError) as e:
            raise RuntimeError(
                f"Failed to process trust domain {trust_domain}: {e}"
            ) from e

    spire_jwks = json.dumps({"keys": all_keys})
    print(
        f"Combined JWKS contains {len(all_keys)} total keys from {len(spire_bundle)} trust domains"
    )

    # Create static JWT provider using the UniFFI function
    provider = slim.create_identity_provider_static_jwt(path=jwt_path)

    # Create JWKS key for verification using the helper function
    key = slim.create_key_with_jwks(content=spire_jwks)

    # Create JWT verifier with the public key
    verifier = slim.create_identity_verifier_jwt(
        public_key=key,
        autoresolve=None,
        issuer=iss,
        audience=aud,
        subject=sub,
        require_iss=None,
        require_aud=None,
        require_sub=None,
    )
    return provider, verifier


def spire_identity(
    socket_path: str | None,
    target_spiffe_id: str | None,
    jwt_audiences: list[str] | None,
):
    """
    Construct a SPIRE-based dynamic identity provider and verifier.

    Args:
        socket_path: SPIRE Workload API socket path (optional).
        target_spiffe_id: Specific SPIFFE ID to request (optional).
        jwt_audiences: Audience list for JWT SVID requests (optional).
    """
    provider = slim.create_identity_provider_spire(
        socket_path=socket_path,
        target_spiffe_id=target_spiffe_id,
        jwt_audiences=list(jwt_audiences) if jwt_audiences else None,
    )
    verifier = slim.create_identity_verifier_spire(
        socket_path=socket_path,
        target_spiffe_id=target_spiffe_id,
        jwt_audiences=list(jwt_audiences) if jwt_audiences else None,
    )
    return provider, verifier


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
        "--jwt",
        type=str,
        help="Static JWT token path for authentication.",
    )(function)

    function = click.option(
        "--spire-trust-bundle",
        type=str,
        help="SPIRE trust bundle path (for static JWT + JWKS mode).",
    )(function)

    function = click.option(
        "--audience",
        type=str,
        help="Audience (comma-separated or single) for static JWT verification.",
    )(function)

    # SPIRE dynamic identity options.
    function = click.option(
        "--spire-socket-path",
        type=str,
        help="SPIRE Workload API socket path (overrides default).",
    )(function)

    function = click.option(
        "--spire-target-spiffe-id",
        type=str,
        help="Target SPIFFE ID to request from SPIRE.",
    )(function)

    function = click.option(
        "--spire-jwt-audience",
        type=str,
        multiple=True,
        help="Audience(s) for SPIRE JWT SVID requests. Can be specified multiple times.",
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
    jwt: str | None = None,
    spire_trust_bundle: str | None = None,
    audience: str | None = None,
    spire_socket_path: str | None = None,
    spire_target_spiffe_id: str | None = None,
    spire_jwt_audience: tuple[str, ...] | None = None,
):
    """
    Build and connect a Slim application instance given user CLI parameters.

    Resolution precedence for auth:
      1. If spire_socket_path or spire_target_spiffe_id or spire_jwt_audience provided -> SPIRE dynamic flow.
      2. Else if jwt + bundle + audience provided -> JWT/JWKS flow.
      3. Else -> shared secret (must be provided, raises if missing).

    Args:
        local: Local identity string (org/ns/app).
        slim_config: Dict of connection parameters (endpoint, tls flags, etc.).
        remote: Optional remote identity (unused here, reserved for future).
        enable_opentelemetry: Enable OTEL tracing export (note: tracing config not available in uniffi).
        shared_secret: Symmetric secret for shared-secret mode.
        jwt: Path to static JWT token (for StaticJwt provider).
        spire_trust_bundle: Path to a spire trust bundle file (containing the JWKs for each trust domain).
        audience: Audience string for JWT verification (comma-separated or single).
        spire_socket_path: SPIRE Workload API socket path (optional).
        spire_target_spiffe_id: Specific SPIFFE ID to request (optional).
        spire_jwt_audience: Audience tuple for SPIRE JWT SVID requests (optional).

    Returns:
        tuple: (app, connection_id) - Connected Slim app instance and connection ID.
    """
    # Initialize crypto provider
    slim.initialize_crypto_provider()

    # Convert local identifier to a strongly typed Name.
    local_name = split_id(local)

    # Derive identity provider & verifier using precedence rules
    if spire_socket_path or spire_target_spiffe_id or spire_jwt_audience:
        print("Using SPIRE dynamic identity authentication.")
        provider, verifier = spire_identity(
            socket_path=spire_socket_path,
            target_spiffe_id=spire_target_spiffe_id,
            jwt_audiences=list(spire_jwt_audience) if spire_jwt_audience else None,
        )
        # Create app with provider and verifier
        app = slim.create_app(local_name, provider, verifier)
    elif jwt and spire_trust_bundle and audience:
        print("Using JWT + JWKS authentication.")
        # Parse audience string (could be comma-separated)
        aud_list = [a.strip() for a in audience.split(",")] if audience else None
        provider, verifier = jwt_identity(
            jwt_path=jwt,
            spire_bundle_path=spire_trust_bundle,
            aud=aud_list,
        )
        # Create app with provider and verifier
        app = slim.create_app(local_name, provider, verifier)
    else:
        print("Using shared-secret authentication.")
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
