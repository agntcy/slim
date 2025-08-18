# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Build container with cross-compilation support
FROM --platform=${BUILDPLATFORM} ghcr.io/astral-sh/uv:0.7.0-python3.13-bookworm AS builder

SHELL ["/bin/bash", "-c"]

ARG TARGETARCH

ENV PATH="/root/.cargo/bin:${PATH}"
ENV PYTHONUNBUFFERED=1
ENV PYTHONDONTWRITEBYTECODE=1
ENV UV_LINK_MODE=copy \
    UV_COMPILE_BYTECODE=1 \
    UV_PYTHON=python3.13 \
    UV_PROJECT_ENVIRONMENT=/app

# Install dependencies required for building rust and Python bindings
RUN DEBIAN_FRONTEND=noninteractive \
    apt-get update && \
    apt-get install --no-install-recommends -y \
        curl \
        file \
        make \
        unzip \
        git \
        pkg-config \
        clang-14 \
        llvm-14

# Install rust toolchain
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

# Install taskfile
RUN sh -c "$(curl --location https://taskfile.dev/install.sh)" -- -d -b /usr/local/bin

# Copy code into builder
COPY . /src
WORKDIR /src/data-plane/python-bindings

# Setup cross-compilation toolchain and Rust targets
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
rustup target add ${RUSTARCH}
cargo fetch --target ${RUSTARCH}
EOF

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

uv python install cpython-3.13.3-linux-${RUSTARCH}
export PYO3_CROSS_INCLUDE_DIR=~/.local/share/uv/python/cpython-3.13.3-linux-${RUSTARCH}-gnu/include
export PYO3_CROSS_LIB_DIR=~/.local/share/uv/python/cpython-3.13.3-linux-${RUSTARCH}-gnu/lib
export CARGO_BUILD_TARGET=${RUSTARCH}-unknown-linux-gnu
task -v python-bindings:examples:build TARGET=${RUSTARCH}-unknown-linux-gnu PROFILE=release
EOF

FROM python:3.13-bookworm AS slim-bindings-examples

ARG TARGETARCH

# Copy venv from builder with just the dependencies we need + our package
COPY --from=builder --chown=app:app /app /app

ENTRYPOINT ["/app/bin/examples"]
