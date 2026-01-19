# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Shared helper utilities for the slim_bindings CLI examples.

This module centralizes:
  * Pretty-print / color formatting helpers
  * Identity (auth) helper constructors (shared secret / JWT / JWKS / SPIRE)
  * Convenience coroutine for constructing a local Slim app using global service
  * Configuration parsing utilities

The heavy inline commenting is intentional: it is meant to teach newcomers
exactly what each step does, line by line.
"""

import argparse
import base64  # Used to decode base64-encoded JWKS content (when provided).
import datetime  # Used for timedelta in JWT configs
import json  # Used for parsing JWKS JSON and dynamic option values.
from typing import Any

import slim_bindings  # The Python bindings package we are demonstrating.

from .config import AuthMode, BaseConfig


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


def setup_service(enable_opentelemetry: bool = False) -> slim_bindings.Service:
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

    # Get the global service instance
    service = slim_bindings.get_global_service()

    return service


async def create_local_app(config: BaseConfig) -> tuple[slim_bindings.App, int]:
    """
    Build a Slim application instance using the global service.

    Resolution precedence for auth:
      1. If SPIRE options provided -> SPIRE dynamic identity flow.
      2. Else if jwt + bundle + audience provided -> JWT/JWKS flow.
      3. Else -> shared secret (must be provided).

    Args:
        config: BaseConfig instance containing all configuration.

    Returns:
        tuple[App, int]: Slim application instance and connection ID.
    """
    # Initialize tracing and global state
    service = setup_service()

    # Convert local identifier to a strongly typed Name.
    local_name = split_id(config.local)

    client_config = slim_bindings.new_insecure_client_config(config.slim)
    conn_id = await service.connect_async(client_config)

    # Determine authentication mode
    auth_mode = config.get_auth_mode()

    if auth_mode == AuthMode.SPIRE:
        print("Using SPIRE dynamic identity authentication.")
        provider_config, verifier_config = spire_identity(
            socket_path=config.spire_socket_path,
            target_spiffe_id=config.spire_target_spiffe_id,
            jwt_audiences=config.spire_jwt_audience,
        )
        local_app = service.create_app(local_name, provider_config, verifier_config)
    elif auth_mode == AuthMode.JWT:
        print("Using JWT + JWKS authentication.")
        # These should always be set if auth_mode is JWT
        if not config.jwt or not config.spire_trust_bundle:
            raise ValueError(
                "JWT and SPIRE trust bundle are required for JWT auth mode"
            )
        provider_config, verifier_config = jwt_identity(
            config.jwt,
            config.spire_trust_bundle,
            str(local_name),
            aud=config.audience,
        )
        local_app = service.create_app(local_name, provider_config, verifier_config)
    else:
        print("Using shared-secret authentication.")
        local_app = service.create_app_with_secret(local_name, config.shared_secret)

    # Provide feedback to user (instance numeric id).
    format_message_print(f"{local_app.id()}", "Created app")

    # Subscribe to the local name
    await local_app.subscribe_async(local_name, conn_id)

    return local_app, conn_id


def create_base_parser(description: str) -> argparse.ArgumentParser:
    """
    Create an ArgumentParser with common options for all examples.

    Args:
        description: Description for the command.

    Returns:
        Configured ArgumentParser instance.
    """
    parser = argparse.ArgumentParser(
        description=description,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )

    # Core identity settings
    parser.add_argument(
        "--local",
        type=str,
        required=True,
        help="Local ID in the format organization/namespace/application",
    )

    parser.add_argument(
        "--remote",
        type=str,
        help="Remote ID in the format organization/namespace/application-or-stream",
    )

    # Service connection
    parser.add_argument(
        "--slim",
        type=str,
        default="http://127.0.0.1:46357",
        help="SLIM remote endpoint URL (default: http://127.0.0.1:46357)",
    )

    # Feature flags
    parser.add_argument(
        "--enable-opentelemetry",
        action="store_true",
        help="Enable OpenTelemetry tracing",
    )

    parser.add_argument(
        "--enable-mls",
        action="store_true",
        help="Enable MLS (Message Layer Security) for sessions",
    )

    # Shared secret authentication
    parser.add_argument(
        "--shared-secret",
        type=str,
        default="abcde-12345-fedcb-67890-deadc",
        help="Shared secret for authentication (development only)",
    )

    # JWT authentication
    parser.add_argument(
        "--jwt",
        type=str,
        help="Path to static JWT token file for authentication",
    )

    parser.add_argument(
        "--spire-trust-bundle",
        type=str,
        help="Path to SPIRE trust bundle file (for JWT + JWKS mode)",
    )

    parser.add_argument(
        "--audience",
        type=str,
        help="Audience for JWT verification (comma-separated)",
    )

    # SPIRE dynamic identity
    parser.add_argument(
        "--spire-socket-path",
        type=str,
        help="SPIRE Workload API socket path",
    )

    parser.add_argument(
        "--spire-target-spiffe-id",
        type=str,
        help="Target SPIFFE ID to request from SPIRE",
    )

    parser.add_argument(
        "--spire-jwt-audience",
        type=str,
        action="append",
        dest="spire_jwt_audience",
        help="Audience(s) for SPIRE JWT SVID requests (can be specified multiple times)",
    )

    # Config file
    parser.add_argument(
        "--config",
        type=str,
        help="Path to configuration file (JSON, YAML, or TOML)",
    )

    return parser


def parse_args_to_dict(args: argparse.Namespace) -> dict[str, Any]:
    """
    Convert argparse Namespace to dictionary, handling special parsing.

    Args:
        args: Parsed arguments from argparse.

    Returns:
        Dictionary of parsed arguments.
    """
    args_dict = vars(args)

    # Parse audience from comma-separated string
    if args_dict.get("audience") and isinstance(args_dict["audience"], str):
        args_dict["audience"] = [
            a.strip() for a in args_dict["audience"].split(",") if a.strip()
        ]

    return args_dict
