# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import base64
import json
import logging

import click

import slim_bindings

logger = logging.getLogger(__name__)

from srpc.rpc import Rpc


class color:
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

# Split an ID into its components
# Expected format: organization/namespace/application
# Raises ValueError if the format is incorrect
# Returns a PyName with the 3 components
def split_id(id):
    try:
        organization, namespace, app = id.split("/")
    except ValueError as e:
        print("Error: IDs must be in the format organization/namespace/app-or-stream.")
        raise e

    return slim_bindings.PyName(organization, namespace, app)


def method_to_pyname(
    name: slim_bindings.PyName, service_name: str, method_name: str
) -> slim_bindings.PyName:
    """
    Convert a method name to a PyName.
    """

    components = name.components_strings()

    if len(components) < 3:
        raise ValueError("PyName must have at least 3 components.")

    subscription_name = slim_bindings.PyName(
        components[0],
        components[1],
        f"{components[2]}-{service_name}-{method_name}",
    )

    logger.debug(f"Name after conversion from service/method: {subscription_name}")

    return subscription_name

def service_and_method_to_pyname(name: slim_bindings.PyName, service_method: str) -> slim_bindings.PyName:
    """
    Convert a method name to a PyName.
    """

    # Split method in service and method name
    service_name = service_method.split("/")[1]
    method_name = service_method.split("/")[2]

    return method_to_pyname(name, service_name, method_name)

def handler_name_to_pyname(
    name: slim_bindings.PyName, rpc_handler: Rpc
) -> slim_bindings.PyName:
    """
    Convert a handler name to a PyName.
    """

    return method_to_pyname(name, rpc_handler.service_name, rpc_handler.method_name)


# Create a shared secret identity provider and verifier
# This is used for shared secret authentication
# Takes an identity and a shared secret as parameters
# Returns a tuple of (provider, verifier)
# This is used for shared secret authentication
def shared_secret_identity(identity, secret):
    """
    Create a provider and verifier using a shared secret.
    """
    provider = slim_bindings.PyIdentityProvider.SharedSecret(
        identity=identity, shared_secret=secret
    )
    verifier = slim_bindings.PyIdentityVerifier.SharedSecret(
        identity=identity, shared_secret=secret
    )

    return provider, verifier


# Create a JWT identity provider and verifier
# This is used for JWT authentication
# Takes private key path, public key path, and algorithm as parameters
# Returns a Slim object with the provider and verifier
def jwt_identity(
    jwt_path: str,
    jwk_path: str,
    iss: str = None,
    sub: str = None,
    aud: list = None,
):
    """
    Parse the JWK and JWT from the provided strings.
    """

    print(f"Using JWk file: {jwk_path}")

    with open(jwk_path) as jwk_file:
        jwk_string = jwk_file.read()

    # The JWK is normally encoded as base64, so we need to decode it
    spire_jwks = json.loads(jwk_string)

    for _, v in spire_jwks.items():
        # Decode first item from base64
        spire_jwks = base64.b64decode(v)
        break

    provider = slim_bindings.PyIdentityProvider.StaticJwt(
        path=jwt_path,
    )

    pykey = slim_bindings.PyKey(
        algorithm=slim_bindings.PyAlgorithm.RS256,
        format=slim_bindings.PyKeyFormat.Jwks,
        key=slim_bindings.PyKeyData.Content(content=spire_jwks.decode("utf-8")),
    )

    verifier = slim_bindings.PyIdentityVerifier.Jwt(
        public_key=pykey,
        issuer=iss,
        audience=aud,
        subject=sub,
    )

    return provider, verifier


# A custom click parameter type for parsing dictionaries from JSON strings
# This is useful for passing complex configurations via command line arguments
class DictParamType(click.ParamType):
    name = "dict"

    def convert(self, value, param, ctx):
        import json

        if isinstance(value, dict):
            return value  # Already a dict (for default value)
        try:
            return json.loads(value)
        except json.JSONDecodeError:
            self.fail(f"{value} is not valid JSON", param, ctx)


def common_options(function):
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
    )(function)

    function = click.option(
        "--jwt",
        type=str,
        help="JWT token for authentication.",
    )(function)

    function = click.option(
        "--bundle",
        type=str,
        help="Key bundle for authentication, in JWKS format.",
    )(function)

    function = click.option(
        "--audience",
        type=str,
        help="Audience for the JWT.",
    )(function)

    function = click.option(
        "--invites",
        type=str,
        multiple=True,
        help="Invite other participants to the pubsub session. Can be specified multiple times.",
    )(function)

    function = click.option(
        "--enable-mls",
        is_flag=True,
        help="Enable MLS (Message Layer Security) for the pubsub session.",
    )(function)

    return function


async def create_local_app(
    local_name: slim_bindings.PyName,
    slim: dict,
    enable_opentelemetry: bool = False,
    shared_secret: str | None = None,
):
    # init tracing
    slim_bindings.init_tracing(
        {
            "log_level": "info",
            "opentelemetry": {
                "enabled": enable_opentelemetry,
                "grpc": {
                    "endpoint": "http://localhost:4317",
                },
            },
        }
    )

    provider, verifier = shared_secret_identity(
        identity=str(local_name),
        secret=shared_secret,
    )

    local_app = await slim_bindings.Slim.new(local_name, provider, verifier)

    logging.info(f"{local_app.get_id()} Created app")

    # Connect to slim server
    _ = await local_app.connect(slim)

    logging.info(f"{local_app.get_id()} Connected to {slim['endpoint']}")

    return local_app
