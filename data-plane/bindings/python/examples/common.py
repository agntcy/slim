# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Shared helper utilities for the slim_bindings CLI examples.

This module centralizes:
  * Pretty-print / color formatting helpers
  * Identity (auth) helper constructors (shared secret / JWT / JWKS / SPIRE)
  * Command-line option decoration (Click integration)
  * Convenience coroutine for constructing a local Slim app using global service

The heavy inline commenting is intentional: it is meant to teach newcomers
exactly what each step does, line by line.
"""

import base64  # Used to decode base64-encoded JWKS content (when provided).
import datetime  # Used for timedelta in JWT configs
import json  # Used for parsing JWKS JSON and dynamic option values.

import click  # CLI option parsing & command composition library.

import slim_bindings  # The Python bindings package we are demonstrating.


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


def split_id(id: str) -> slim_bindings.Name:
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
        organization, namespace, app = id.split("/")
    except ValueError as e:
        print("Error: IDs must be in the format organization/namespace/app-or-stream.")
        raise e
    return slim_bindings.Name(organization, namespace, app)


def jwt_identity(
    jwt_path: str,
    spire_bundle_path: str,
    local_name: str,
    iss: str | None = None,
    sub: str | None = None,
    aud: list[str] | None = None,
):
    """
    Construct a JWT provider and verifier from file inputs.

    Process:
      1. Read a JSON structure containing (base64-encoded) JWKS data (a SPIRE
         bundle with a JWKS for each trust domain).
      2. Decode & merge all JWKS entries.
      3. Create a JWT identity provider with static file JWT.
      4. Wrap merged JWKS JSON as JwtKeyConfig with RS256 & JWKS format.
      5. Build a JWT verifier using the JWKS-derived public key.
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

    # Read the static JWT file for signing
    with open(jwt_path) as jwt_file:
        jwt_content = jwt_file.read()

    # Create encoding key config for JWT signing
    encoding_key_config = slim_bindings.JwtKeyConfig(
        algorithm=slim_bindings.JwtAlgorithm.RS256,
        format=slim_bindings.JwtKeyFormat.PEM,
        key=slim_bindings.JwtKeyData.DATA(value=jwt_content),
    )

    # Create provider config for JWT authentication
    provider_config = slim_bindings.IdentityProviderConfig.JWT(
        config=slim_bindings.ClientJwtAuth(
            key=slim_bindings.JwtKeyType.ENCODING(key=encoding_key_config),
            audience=aud or ["default-audience"],
            issuer=iss or "default-issuer",
            subject=sub or local_name,
            duration=datetime.timedelta(seconds=3600),
        )
    )

    # Create decoding key config for JWKS verification
    decoding_key_config = slim_bindings.JwtKeyConfig(
        algorithm=slim_bindings.JwtAlgorithm.RS256,
        format=slim_bindings.JwtKeyFormat.JWKS,
        key=slim_bindings.JwtKeyData.DATA(value=spire_jwks),
    )

    # Create verifier config
    verifier_config = slim_bindings.IdentityVerifierConfig.JWT(
        config=slim_bindings.JwtAuth(
            key=slim_bindings.JwtKeyType.DECODING(key=decoding_key_config),
            audience=aud or ["default-audience"],
            issuer=iss or "default-issuer",
            subject=sub,
            duration=datetime.timedelta(seconds=3600),
        )
    )
    return provider_config, verifier_config


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
    spire_config = slim_bindings.SpireConfig(
        trust_domains=[],
        socket_path=socket_path,
        target_spiffe_id=target_spiffe_id,
        jwt_audiences=list(jwt_audiences) if jwt_audiences else [],
    )

    provider_config = slim_bindings.IdentityProviderConfig.SPIRE(config=spire_config)
    verifier_config = slim_bindings.IdentityVerifierConfig.SPIRE(config=spire_config)

    return provider_config, verifier_config


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
    enable_opentelemetry: bool = False,
    shared_secret: str = "abcde-12345-fedcb-67890-deadc",
    jwt: str | None = None,
    spire_trust_bundle: str | None = None,
    audience: list[str] | None = None,
    spire_socket_path: str | None = None,
    spire_target_spiffe_id: str | None = None,
    spire_jwt_audience: list[str] | None = None,
):
    """
    Build a Slim application instance using the global service.

    Resolution precedence for auth:
      1. If SPIRE options provided -> SPIRE dynamic identity flow.
      2. Else if jwt + bundle + audience provided -> JWT/JWKS flow.
      3. Else -> shared secret (must be provided).

    Args:
        local: Local identity string (org/ns/app).
        enable_opentelemetry: Enable OTEL tracing export.
        shared_secret: Symmetric secret for shared-secret mode.
        jwt: Path to static JWT token (for JWT provider).
        spire_trust_bundle: Path to a spire trust bundle file (containing the JWKs for each trust domain).
        audience: Audience list for JWT verification.
        spire_socket_path: SPIRE Workload API socket path (optional).
        spire_target_spiffe_id: Specific SPIFFE ID to request (optional).
        spire_jwt_audience: Audience list for JWT SVID requests (optional).

    Returns:
        App: Slim application instance using the global service.
    """
    # Initialize tracing and global state
    tracing_config = slim_bindings.new_tracing_config()
    runtime_config = slim_bindings.new_runtime_config()
    service_config = slim_bindings.new_service_config()

    tracing_config.log_level = "info"

    if enable_opentelemetry:
        # Note: OpenTelemetry configuration through config objects is complex
        # For now, we'll just initialize with default tracing
        # Users can set OTEL environment variables for full OTEL support
        pass

    slim_bindings.initialize_with_configs(
        tracing_config=tracing_config,
        runtime_config=runtime_config,
        service_config=[service_config],
    )

    # Convert local identifier to a strongly typed Name.
    local_name = split_id(local)

    # Get the global service instance
    service = slim_bindings.get_global_service()

    # Derive identity provider & verifier using SPIRE if options supplied.
    if spire_socket_path or spire_target_spiffe_id or spire_jwt_audience:
        print("Using SPIRE dynamic identity authentication.")
        provider_config, verifier_config = spire_identity(
            socket_path=spire_socket_path,
            target_spiffe_id=spire_target_spiffe_id,
            jwt_audiences=spire_jwt_audience,
        )
        # Create app with full auth config
        local_app = service.create_app(local_name, provider_config, verifier_config)
    # Or use JWT/JWKS if all pieces supplied.
    elif jwt and spire_trust_bundle and audience:
        print("Using JWT + JWKS authentication.")
        # Parse audience if it's a comma-separated string
        aud_list = audience.split(",") if isinstance(audience, str) else audience
        provider_config, verifier_config = jwt_identity(
            jwt,
            spire_trust_bundle,
            str(local_name),
            aud=aud_list,
        )
        # Create app with full auth config
        local_app = service.create_app(local_name, provider_config, verifier_config)
    else:
        print("Using shared-secret authentication.")
        # Use the convenience method for shared secret
        local_app = service.create_app_with_secret(local_name, shared_secret)

    # Provide feedback to user (instance numeric id).
    format_message_print(f"{local_app.id()}", "Created app")

    return local_app
