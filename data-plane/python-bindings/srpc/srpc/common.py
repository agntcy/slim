# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import base64
import json
import logging
from typing import Tuple, TypeVar

import slim_bindings

logger = logging.getLogger(__name__)

DEADLINE_KEY = "srpc-timeout"

MAX_TIMEOUT = 36000  # 10h

# Types for SRPC API
RequestType = TypeVar("RequestType")
ResponseType = TypeVar("ResponseType")


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
def split_id(id: str) -> slim_bindings.PyName:
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


def service_and_method_to_pyname(
    name: slim_bindings.PyName, service_method: str
) -> slim_bindings.PyName:
    """
    Convert a method name to a PyName.
    """

    # Split method in service and method name
    service_name = service_method.split("/")[1]
    method_name = service_method.split("/")[2]

    return method_to_pyname(name, service_name, method_name)


def handler_name_to_pyname(
    name: slim_bindings.PyName,
    service_name: str,
    method_name: str,
) -> slim_bindings.PyName:
    """
    Convert a handler name to a PyName.
    """

    return method_to_pyname(name, service_name, method_name)


# Create a shared secret identity provider and verifier
# This is used for shared secret authentication
# Takes an identity and a shared secret as parameters
# Returns a tuple of (provider, verifier)
# This is used for shared secret authentication
def shared_secret_identity(
    identity: str, secret: str
) -> Tuple[slim_bindings.PyIdentityProvider, slim_bindings.PyIdentityVerifier]:
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
    iss: str | None = None,
    sub: str | None = None,
    aud: list | None = None,
) -> Tuple[slim_bindings.PyIdentityProvider, slim_bindings.PyIdentityVerifier]:
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


async def create_local_app(
    local_name: slim_bindings.PyName,
    slim: dict,
    enable_opentelemetry: bool = False,
    shared_secret: str = "",
) -> slim_bindings.Slim:
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

    logger.info(f"{local_app.get_id()} Created app")

    # Connect to slim server
    _ = await local_app.connect(slim)

    logger.info(f"{local_app.get_id()} Connected to {slim['endpoint']}")

    return local_app
