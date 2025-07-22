# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import slim_bindings


async def create_svc(organization, namespace, agent_type, secret):
    provider = slim_bindings.PyIdentityProvider.SharedSecret(
        identity=agent_type, shared_secret=secret
    )
    verifier = slim_bindings.PyIdentityVerifier.SharedSecret(
        identity=agent_type, shared_secret=secret
    )
    return await slim_bindings.create_pyservice(
        organization, namespace, agent_type, provider, verifier
    )


async def create_slim(organization, namespace, agent_type, secret):
    provider = slim_bindings.PyIdentityProvider.SharedSecret(
        identity=agent_type, shared_secret=secret
    )
    verifier = slim_bindings.PyIdentityVerifier.SharedSecret(
        identity=agent_type, shared_secret=secret
    )
    return await slim_bindings.Slim.new(
        organization, namespace, agent_type, provider, verifier
    )
