# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""
SLIM UniFFI Python Bindings

This package provides Python bindings for SLIM (Secure Low-Latency Interactive Messaging)
generated from Rust using UniFFI.
"""

__version__ = "0.7.0"

# Re-export all generated bindings from Maturin-generated UniFFI module
from ._slim_bindings.slim_bindings import *  # noqa: F403, F401

__all__ = ["__version__"]
