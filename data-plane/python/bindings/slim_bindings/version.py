# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from slim_bindings._slim_bindings import (  # type: ignore[attr-defined]
    __version__,
    build_info,
    build_profile,
)


def get_version():
    """
    Get the version of the SLIM bindings.

    Returns:
        str: The version of the SLIM bindings.
    """
    return __version__


def get_build_profile():
    """
    Get the build profile of the SLIM bindings.

    Returns:
        str: The build profile of the SLIM bindings.
    """
    return build_profile


def get_build_info():
    """
    Get the build information of the SLIM bindings.

    Returns:
        str: The build information of the SLIM bindings.
    """
    return build_info
