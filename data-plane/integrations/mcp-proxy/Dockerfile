# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Build container
FROM --platform=${BUILDPLATFORM} rust:1.87-slim-bookworm AS rust

SHELL ["/bin/bash", "-c"]

ARG TARGETARCH

RUN DEBIAN_FRONTEND=noninteractive \
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
        pkg-config && \
    curl -L -o /tmp/llvm.sh https://apt.llvm.org/llvm.sh && \
    curl -1sLf 'https://dl.cloudsmith.io/public/task/task/setup.deb.sh' | bash && \
    chmod +x /tmp/llvm.sh && \
    /tmp/llvm.sh 19 && \
    apt-get install -y go-task

# Copy source code
COPY . /app
WORKDIR /app/data-plane/integrations/mcp-proxy

RUN <<EOF
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

apt-get update && apt-get install -y ${PACKAGES}

# Fetch rust packages
task -v mcp-proxy:fetch TARGET=${RUSTARCH}
EOF

# Build debug application
RUN <<EOF
case ${TARGETARCH} in
    "amd64")
        RUSTARCH=x86_64
        ;;
    "arm64")
        RUSTARCH=aarch64
        ;;
    *)
        echo "Unsupported platform: ${TARGETPLATFORM}"
        exit 1
        ;;
esac

task -v mcp-proxy:build:strip TARGET=${RUSTARCH}-unknown-linux-gnu PROFILE=release
mv target/${RUSTARCH}-unknown-linux-gnu target/${TARGETARCH}-unknown-linux-gnu
EOF

FROM ghcr.io/distroless/cc-debian12:nonroot AS mcp-proxy-debug
ARG TARGETARCH

# copy the artifacts from the build stage
COPY --from=rust /app/data-plane/integrations/mcp-proxy/target/${TARGETARCH}-unknown-linux-gnu/release/slim-mcp-proxy /slim-mcp-proxy
COPY --from=rust /app/data-plane/integrations/mcp-proxy/target/${TARGETARCH}-unknown-linux-gnu/release/slim-mcp-proxy.dbg /slim-mcp-proxy.dbg

FROM gcr.io/distroless/cc-debian12:nonroot AS mcp-proxy-release
ARG TARGETARCH

# copy the artifacts from the build stage
COPY --from=rust /app/data-plane/integrations/mcp-proxy/target/${TARGETARCH}-unknown-linux-gnu/release/slim-mcp-proxy /slim-mcp-proxy
