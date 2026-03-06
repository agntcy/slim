#!/bin/sh
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
#
# Dispatcher for slim-bindings examples.
# Usage: docker run <image> <example> [args...]
#   example: group, p2p, or p2 (alias for p2p)

set -e

case "$1" in
  group)
    shift
    exec slim-bindings-group "$@"
    ;;
  p2p|p2)
    shift
    exec slim-bindings-p2p "$@"
    ;;
  *)
    echo "Unknown example: $1. Use: group, p2p, or p2"
    exit 1
    ;;
esac
