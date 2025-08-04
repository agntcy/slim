# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

FROM ghcr.io/astral-sh/uv:0.7.0-python3.13-bookworm AS builder

ENV PATH="/root/.cargo/bin:${PATH}"

ENV PYTHONUNBUFFERED=1
ENV PYTHONDONTWRITEBYTECODE=1

ENV UV_LINK_MODE=copy \
    UV_COMPILE_BYTECODE=1 \
    UV_PYTHON_DOWNLOADS=never \
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

RUN ls /root/.cargo/bin

# Install just the slim-bindings-examples package
RUN task -v python-bindings:examples:build PROFILE=release

FROM python:3.13-bookworm AS slim-bindings-examples

# Copy venv from builder with just the dependencies we need + our package
COPY --from=builder --chown=app:app /app /app

ENTRYPOINT ["/app/bin/examples"]
