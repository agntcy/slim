# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Build container
FROM --platform=${BUILDPLATFORM} rust:1.90-slim-bookworm AS rust

SHELL ["/bin/bash", "-c"]

RUN --mount=type=cache,id=target-$TARGETARCH,sharing=locked,target=/app/data-plane/target <<EOF
echo "cache test" > /app/data-plane/target/test.txt
EOF
