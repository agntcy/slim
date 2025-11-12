# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import slim_bindings
from slim_bindings._slim_bindings import PyApp


def create_svc(
    name: slim_bindings.PyName,
    secret: str = "testing-secret-123456789012345abc",
    local_service: bool = True,
):
    """Create and return a low-level PyService for tests.

    Sets up a SharedSecret-based identity provider and verifier with the same
    secret so that authentication succeeds without external infrastructure.

    Args:
        name: Fully qualified PyName identifying the local service/app.
        secret: Shared secret string used for symmetric token generation/verification.

    Returns:
        PyService: The underlying service handle usable with session creation
        and message operations.
    """

    provider = slim_bindings.PyIdentityProvider.SharedSecret(  # type: ignore
        identity=f"{name}", shared_secret=secret
    )
    verifier = slim_bindings.PyIdentityVerifier.SharedSecret(  # type: ignore
        identity=f"{name}", shared_secret=secret
    )
    return PyApp(name, provider, verifier, local_service=local_service)


def create_slim(
    name: slim_bindings.PyName,
    secret: str = "testing-secret-123456789012345abc",
    local_service: bool = True,
):
    """Create and return a high-level Slim instance for tests.

    This wraps the same SharedSecret authentication setup as create_svc but
    returns the Slim abstraction, giving access to convenience methods such
    as create_session, connect, subscribe, etc.

    Args:
        name: Fully qualified PyName for the local application/service.
        secret: Shared secret used for symmetric identity provider/verifier.

    Returns:
        Slim: High-level wrapper around the newly created PyService.
    """
    provider = slim_bindings.PyIdentityProvider.SharedSecret(  # type: ignore
        identity=f"{name}", shared_secret=secret
    )
    verifier = slim_bindings.PyIdentityVerifier.SharedSecret(  # type: ignore
        identity=f"{name}", shared_secret=secret
    )
    return slim_bindings.Slim(
        name, provider, verifier, local_service=local_service
    )
