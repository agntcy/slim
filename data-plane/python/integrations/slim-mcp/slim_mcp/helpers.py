# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import base64
import json
import logging
from typing import Tuple, TypeVar

import slim_bindings

logger = logging.getLogger(__name__)

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
