# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Build container with cross-compilation support
FROM --platform=${BUILDPLATFORM} debian:bookworm AS builder

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

ARG TARGETARCH

ENV PATH="/root/.cargo/bin:/root/.local/bin:${PATH}"
ENV PYTHONUNBUFFERED=1
ENV PYTHONDONTWRITEBYTECODE=1
ENV UV_LINK_MODE=copy \
    UV_COMPILE_BYTECODE=1 \
    UV_PYTHON=python3.14 \
    UV_PROJECT_ENVIRONMENT=/app

# Install dependencies required for building rust and Python bindings
RUN --mount=type=cache,target=/app/data-plane/target <<EOF
DEBIAN_FRONTEND=noninteractive \
    apt-get update && \
    apt-get install --no-install-recommends -y \
        curl \
        git \
        ca-certificates \
        gcc \
        make \
        libc-dev

curl -LsSf https://astral.sh/uv/install.sh | sh

curl -1sLf 'https://dl.cloudsmith.io/public/task/task/setup.deb.sh' | bash
apt-get install -y task

# Install rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
EOF

# Copy code into builder
COPY . /src
WORKDIR /src/data-plane/python/bindings

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

export CC_x86_64_unknown_linux_gnu=cc
export AR_x86_64_unknown_linux_gnu=ar
export STRIP_x86_64_unknown_linux_gnu=strip

TARGET=${RUSTARCH}-unknown-linux-gnu
task python:bindings:packaging TARGET=${TARGET} PROFILE=release INTERPRETERS="3.14"
uv build --package slim-bindings-examples
EOF

FROM python:3.14-bookworm AS slim-bindings-examples

ARG TARGETARCH

# Copy venv from builder with just the dependencies we need + our package
COPY --from=builder --chown=app:app /src/data-plane/python/bindings/dist /dists
RUN python -m pip install /dists/*.whl

ENTRYPOINT ["examples"]
