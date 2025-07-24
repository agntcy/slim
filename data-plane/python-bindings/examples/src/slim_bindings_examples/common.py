# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import json
import os
import slim_bindings
import base64

from jwt import PyJWK


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


def format_message(message1, message2=""):
    return f"{color.BOLD}{color.CYAN}{message1.capitalize():<45}{color.END}{message2}"


def split_id(id):
    # Split the IDs into their respective components
    try:
        local_organization, local_namespace, local_agent = id.split("/")
    except ValueError as e:
        print("Error: IDs must be in the format organization/namespace/agent.")
        raise e

    return local_organization, local_namespace, local_agent


def connection_string(address):
    return {"endpoint": address, "tls": {"insecure": True}}


def get_provider_and_verifier_from_shared_secret(identity, secret):
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


def get_provider_and_verifier_with_spire(
    jwt_path: str,
    jwk_path: str,
    iss: str = None,
    sub: str = None,
    aud: str = None,
):
    """
    Parse the JWK and JWT from the provided strings.
    """

    print(f"Using JWk file: {jwk_path}")

    with open(jwk_path, "r") as jwk_file:
        jwk_string = jwk_file.read()

    # The JWK is normally encoded as base64, so we need to decode it
    spire_jwks = json.loads(jwk_string)

    for k, v in spire_jwks.items():
        # Decode first item from base64
        spire_jwks = base64.b64decode(v)
        spire_jwks = json.loads(spire_jwks)
        spire_jwk = spire_jwks["keys"][0]
        break

    provider = slim_bindings.PyIdentityProvider.StaticJwt(
        path=jwt_path,
    )

    pykey = slim_bindings.PyKey.Jwk(jwk=json.dumps(spire_jwk))

    verifier = slim_bindings.PyIdentityVerifier.Jwt(
        public_key=pykey,
        issuer=iss,
        audience=aud,
        subject=sub,
    )

    return provider, verifier


def common_parser():
    parser = argparse.ArgumentParser(
        description="Command line client for message passing."
    )
    parser.add_argument(
        "-l",
        "--local",
        type=str,
        default=os.getenv("SLIM_LOCAL_ID", "organization/namespace/agent"),
        help="Local ID in the format organization/namespace/agent.",
    )
    parser.add_argument(
        "-r",
        "--remote",
        type=str,
        default=os.getenv("SLIM_REMOTE_ID", "organization/namespace/agent"),
        help="Remote ID in the format organization/namespace/agent.",
    )
    parser.add_argument(
        "-s",
        "--slim",
        type=str,
        help="Slim address.",
        default="http://127.0.0.1:46357",
    )
    parser.add_argument(
        "-t",
        "--enable-opentelemetry",
        action="store_true",
        default=False,
        help="Enable OpenTelemetry tracing.",
    )
    parser.add_argument(
        "-z",
        "--shared-secret",
        type=str,
        default="secret",
        help="Shared secret for authentication. Don't use this in production.",
    )
    parser.add_argument(
        "-j",
        "--jwt",
        type=str,
        help="JWT token for authentication.",
        default=None,
    )
    parser.add_argument(
        "-k",
        "--key",
        type=str,
        help="Key file for authentication, in JWK format.",
        default=None,
    )

    return parser
