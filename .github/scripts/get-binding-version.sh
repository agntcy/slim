#!/usr/bin/env bash
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
#
# Outputs the bindings version from a tag or GITHUB_REF.
# Tag format: slim-bindings-v1.0.0 or slim-bindings-1.0.0 → 1.0.0
# Usage: get-binding-version.sh [ref]
#   If no arg, uses GITHUB_REF env var. Otherwise uses the first argument.

set -euo pipefail

ref="${1:-${GITHUB_REF:-}}"
if [ -z "$ref" ]; then
  echo "Usage: $0 <ref>   OR   set GITHUB_REF" >&2
  exit 1
fi

# Strip refs/tags/ if present
tag="${ref#refs/tags/}"
# Strip tag prefix (slim-bindings- or slim-node-test-bindings- for test tags)
version="${tag#slim-bindings-}"
version="${version#slim-node-test-bindings-}"
version="${version#v}"
echo "$version"
