# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Build container
FROM --platform=${BUILDPLATFORM} rust:1.87-slim-bookworm AS rust

SHELL ["/bin/bash", "-c"]

ARG TARGETARCH

RUN --mount=type=cache,target=/app/data-plane/integrations/mcp-proxy/target <<EOF
case ${TARGETARCH} in
    "amd64")
        PACKAGES="gcc-x86-64-linux-gnu g++-x86-64-linux-gnu"
        RUSTARCH="x86_64-unknown-linux-gnu"
        ;;
    "arm64")
        PACKAGES="gcc-aarch64-linux-gnu g++-aarch64-linux-gnu"
        RUSTARCH="aarch64-unknown-linux-gnu"
        ;;
    *)
        echo "Unsupported platform: ${TARGETPLATFORM}"
        exit 1
        ;;
esac

DEBIAN_FRONTEND=noninteractive \
    apt-get update && \
    apt-get install --no-install-recommends -y \
        curl \
        file \
        make \
        unzip \
        git \
        lsb-release \
        software-properties-common \
        gnupg \
        pkg-config \
        ${PACKAGES}

curl -L -o /tmp/llvm.sh https://apt.llvm.org/llvm.sh
chmod +x /tmp/llvm.sh
/tmp/llvm.sh 19

curl -1sLf 'https://dl.cloudsmith.io/public/task/task/setup.deb.sh' | bash
apt-get install -y task
EOF

# Build debug application
RUN <<EOF
case ${TARGETARCH} in
    "amd64")
        RUSTARCH=x86_64-unknown-linux-gnu
        ;;
    "arm64")
        RUSTARCH=aarch64-unknown-linux-gnu
        ;;
    *)
        echo "Unsupported platform: ${TARGETPLATFORM}"
        exit 1
        ;;
esac

# Fetch rust packages
task -v mcp-proxy:fetch TARGET=${RUSTARCH}

task -v mcp-proxy:build:strip TARGET=${RUSTARCH} PROFILE=release
mv target/${RUSTARCH}/release/slim-mcp-proxy /slim-mcp-proxy
mv target/${RUSTARCH}/release/slim-mcp-proxy.dbg /slim-mcp-proxy.dbg
EOF

FROM ghcr.io/distroless/cc-debian12:nonroot AS mcp-proxy-debug
ARG TARGETARCH

# copy the artifacts from the build stage
COPY --from=rust /slim-mcp-proxy /slim-mcp-proxy
COPY --from=rust /slim-mcp-proxy.dbg /slim-mcp-proxy.dbg

FROM gcr.io/distroless/cc-debian12:nonroot AS mcp-proxy-release
ARG TARGETARCH

# copy the artifacts from the build stage
COPY --from=rust /slim-mcp-proxy /slim-mcp-proxy
