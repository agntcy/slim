# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""Shared constants for slimrpc simple examples.

These values are consistent across all language bindings (Go, Python, Java, C#).
"""

import os

# Default SLIM server endpoint.
SLIM_ADDR = os.environ.get("SLIM_ADDR", "http://localhost:46357")

# Shared secret used for authentication in all examples.
SHARED_SECRET = "my_shared_secret_for_testing_purposes_only"

# Name components used across all examples.
NAME_ORG = "agntcy"
NAME_NS = "grpc"
