# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""
Examples package for SLIM Python bindings.

This package provides example applications demonstrating SLIM functionality:
  - Group messaging (group.py)
  - Point-to-point messaging (point_to_point.py)
  - SLIM server (slim.py)

All examples use Pydantic for configuration, supporting:
  - Command-line arguments
  - Environment variables (SLIM_* prefix)
  - Configuration files (JSON, YAML, TOML via SLIM_CONFIG_FILE env var)

To install dependencies:
    pip install 'slim-bindings[examples]'
"""

import sys
from importlib.util import find_spec

# Check for required dependencies
_missing_deps = []

if find_spec("pydantic") is None:
    _missing_deps.append("pydantic")

if find_spec("pydantic_settings") is None:
    _missing_deps.append("pydantic-settings")

if find_spec("prompt_toolkit") is None:
    _missing_deps.append("prompt-toolkit")

if _missing_deps:
    print(
        f"Missing required dependencies: {', '.join(_missing_deps)}\n"
        "Install them with: pip install 'slim-bindings[examples]'",
        file=sys.stderr,
    )
    sys.exit(1)

__all__ = ["config", "common", "group", "point_to_point", "slim"]
