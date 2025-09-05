# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import slim_bindings


async def create_svc(name: slim_bindings.PyName, secret):
    provider = slim_bindings.PyIdentityProvider.SharedSecret(
        identity=f"{name}", shared_secret=secret
    )
    verifier = slim_bindings.PyIdentityVerifier.SharedSecret(
        identity=f"{name}", shared_secret=secret
    )
    return await slim_bindings.create_pyservice(name, provider, verifier)


async def create_slim(name: slim_bindings.PyName, secret):
    provider = slim_bindings.PyIdentityProvider.SharedSecret(
        identity=f"{name}", shared_secret=secret
    )
    verifier = slim_bindings.PyIdentityVerifier.SharedSecret(
        identity=f"{name}", shared_secret=secret
    )
    return await slim_bindings.Slim.new(name, provider, verifier)
