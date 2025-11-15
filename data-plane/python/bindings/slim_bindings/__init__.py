# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from slim_bindings._slim_bindings import (
    Algorithm,
    App,
    CompletionHandle,
    IdentityProvider,
    IdentityVerifier,
    Key,
    KeyData,
    KeyFormat,
    MessageContext,
    Name,
    SessionConfiguration,
    SessionContext,
    SessionType,
    init_tracing,
)
from slim_bindings.errors import SLIMTimeoutError
from slim_bindings.session import Session
from slim_bindings.slim import Slim
from slim_bindings.version import get_build_info, get_build_profile, get_version

# High-level public API - only expose the main interface and supporting types
__all__ = [
    "get_build_info",
    "get_build_profile",
    "get_version",
    "init_tracing",
    "App",
    "Algorithm",
    "IdentityProvider",
    "IdentityVerifier",
    "Key",
    "KeyData",
    "KeyFormat",
    "MessageContext",
    "Name",
    "Session",
    "SessionConfiguration",
    "SessionContext",
    "SessionType",
    "CompletionHandle",
    "SLIMTimeoutError",
    "Slim",
]
