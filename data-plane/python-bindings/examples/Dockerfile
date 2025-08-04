# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

FROM --platform=${BUILDPLATFORM} ghcr.io/astral-sh/uv:0.7.0-python3.13-bookworm AS builder

ARG TARGETARCH
ARG TARGETOS

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
EOF

# Copy code into builder
COPY . /src
WORKDIR /src/data-plane/python-bindings

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

# Install python for correct target arch
uv python install cpython-3.13-linux-${RUSTARCH}

# Export variables for maturin
export PYO3_CROSS_INCLUDE_DIR=~/.local/share/uv/python/cpython-3.13.3-linux-x86_64-gnu/include
export PYO3_CROSS_LIB_DIR=~/.local/share/uv/python/cpython-3.13.3-linux-x86_64-gnu/lib

# Build examples
task -v python-bindings:examples:build TARGET=${RUSTARCH}-unknown-linux-gnu PROFILE=release
mv target/${RUSTARCH}-unknown-linux-gnu target/${TARGETARCH}-unknown-linux-gnu
EOF

# FROM python:3.13-bookworm AS slim-bindings-examples

# # Copy venv from builder with just the dependencies we need + our package
# COPY --from=builder --chown=app:app /app /app

# ENTRYPOINT ["/app/bin/examples"]
