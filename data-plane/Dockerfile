# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Build container
FROM --platform=${BUILDPLATFORM} rust:1.90-slim-bookworm AS rust

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

ARG TARGETARCH

RUN <<EOF
case ${TARGETARCH} in
    "amd64")
        PACKAGES="gcc-x86-64-linux-gnu g++-x86-64-linux-gnu"
        ;;
    "arm64")
        PACKAGES="gcc-aarch64-linux-gnu g++-aarch64-linux-gnu"
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

# Copy source code
COPY . /app
WORKDIR /app/data-plane

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
task -v data-plane:fetch TARGET=${RUSTARCH}

# Build application
task -v data-plane:build:strip TARGET=${RUSTARCH} PROFILE=release
mv target/${RUSTARCH}/release/slim /slim
mv target/${RUSTARCH}/release/slim.dbg /slim.dbg

# Strip and export control plane binary
task -v data-plane:strip TARGET_BIN=target/${RUSTARCH}/release/slim-control-plane
mv target/${RUSTARCH}/release/slim-control-plane /slim-control-plane
mv target/${RUSTARCH}/release/slim-control-plane.dbg /slim-control-plane.dbg
EOF

# Grab libgcc from the CC image
FROM gcr.io/distroless/cc-debian12 AS libgcc-provider

# Runtime images  - debug executable, debug symbols and, most importantly, a shell :)
FROM debian:bookworm-slim AS slim-debug

ARG TARGETARCH

# copy the build artifacts from the build stage
COPY --from=rust /slim /slim
COPY --from=rust /slim.dbg /slim.dbg

# Runtime images - release executable
FROM gcr.io/distroless/base-nossl-debian12:nonroot AS slim-release

ARG TARGETARCH

# Copy libgcc from the libgcc-provider image
COPY --from=libgcc-provider /lib/*-linux-gnu/libgcc_s.so.1 /lib/

# copy the artifacts from the build stage
COPY --from=rust /slim /slim

# Runtime image - control plane debug executable, debug symbols and a shell
FROM debian:bookworm-slim AS control-plane-debug

ARG TARGETARCH

# copy the build artifacts from the build stage
COPY --from=rust /slim-control-plane /slim-control-plane
COPY --from=rust /slim-control-plane.dbg /slim-control-plane.dbg

# Runtime image - control plane release executable
FROM gcr.io/distroless/base-nossl-debian12:nonroot AS control-plane-release

ARG TARGETARCH

# Copy libgcc from the libgcc-provider image
COPY --from=libgcc-provider /lib/*-linux-gnu/libgcc_s.so.1 /lib/

# copy the artifacts from the build stage
COPY --from=rust /slim-control-plane /slim-control-plane

ENTRYPOINT ["/slim-control-plane"]
