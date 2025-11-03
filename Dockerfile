# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Build container
FROM --platform=${BUILDPLATFORM} rust:1.90-slim-bookworm AS rust

SHELL ["/bin/bash", "-c"]

RUN --mount=type=cache,target=/app/data-plane/target <<EOF
fallocate -l 100M /app/data-plane/target/cache.ims
EOF
