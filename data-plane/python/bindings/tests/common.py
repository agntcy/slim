# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import slim_bindings


def create_svc(
    name: slim_bindings.Name,
    secret: str = "testing-secret-123456789012345abc",
    local_service: bool = True,
):
    """Create and return a low-level App for tests.

    Sets up a SharedSecret-based identity provider and verifier with the same
    secret so that authentication succeeds without external infrastructure.

    Args:
        name: Fully qualified Name identifying the local service/app.
        secret: Shared secret string used for symmetric token generation/verification.

    Returns:
        App: The underlying service handle usable with session creation
        and message operations.
    """

    provider = slim_bindings.IdentityProvider.SharedSecret(
        identity=f"{name}", shared_secret=secret
    )
    verifier = slim_bindings.IdentityVerifier.SharedSecret(
        identity=f"{name}", shared_secret=secret
    )
    return slim_bindings.App(name, provider, verifier, local_service=local_service)


def create_slim(
    name: slim_bindings.Name,
    secret: str = "testing-secret-123456789012345abc",
    local_service: bool = True,
):
    """Create and return a high-level Slim instance for tests.

    This wraps the same SharedSecret authentication setup as create_svc but
    returns the Slim abstraction, giving access to convenience methods such
    as create_session, connect, subscribe, etc.

    Args:
        name: Fully qualified Name for the local application/service.
        secret: Shared secret used for symmetric identity provider/verifier.

    Returns:
        Slim: High-level wrapper around the newly created App.
    """
    provider = slim_bindings.IdentityProvider.SharedSecret(
        identity=f"{name}", shared_secret=secret
    )
    verifier = slim_bindings.IdentityVerifier.SharedSecret(
        identity=f"{name}", shared_secret=secret
    )
    return slim_bindings.Slim(name, provider, verifier, local_service=local_service)
