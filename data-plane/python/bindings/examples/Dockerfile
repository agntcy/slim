# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Build container with cross-compilation support
FROM --platform=${BUILDPLATFORM} ghcr.io/astral-sh/uv:0.7.0-python3.13-bookworm AS builder

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

ARG TARGETARCH

ENV PATH="/root/.cargo/bin:${PATH}"
ENV PYTHONUNBUFFERED=1
ENV PYTHONDONTWRITEBYTECODE=1
ENV UV_LINK_MODE=copy \
    UV_COMPILE_BYTECODE=1 \
    UV_PYTHON=python3.13 \
    UV_PROJECT_ENVIRONMENT=/app

# Install dependencies required for building rust and Python bindings
RUN --mount=type=cache,target=/app/data-plane/target <<EOF
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

# Install rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
EOF

# Copy code into builder
COPY . /src
WORKDIR /src/data-plane/python

# Build the Python bindings with cross-compilation
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

cargo fetch --target ${RUSTARCH}-unknown-linux-gnu

uv python install cpython-3.13.3-linux-${RUSTARCH}
export PYO3_CROSS_INCLUDE_DIR=~/.local/share/uv/python/cpython-3.13.3-linux-${RUSTARCH}-gnu/include
export PYO3_CROSS_LIB_DIR=~/.local/share/uv/python/cpython-3.13.3-linux-${RUSTARCH}-gnu/lib
export CARGO_BUILD_TARGET=${RUSTARCH}-unknown-linux-gnu
task -v python:examples:build TARGET=${RUSTARCH}-unknown-linux-gnu PROFILE=release
rm /app/bin/python && ln -s /usr/local/bin/python /app/bin/python
EOF

FROM python:3.13-bookworm AS slim-bindings-examples

ARG TARGETARCH

# Copy venv from builder with just the dependencies we need + our package
COPY --from=builder --chown=app:app /app /app

ENTRYPOINT ["/app/bin/examples"]
