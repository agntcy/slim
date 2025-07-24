# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

FROM ghcr.io/astral-sh/uv:0.7.0-python3.13-bookworm AS builder

ENV PYTHONUNBUFFERED=1
ENV PYTHONDONTWRITEBYTECODE=1

ENV UV_LINK_MODE=copy \
    UV_COMPILE_BYTECODE=1 \
    UV_PYTHON_DOWNLOADS=never \
    UV_PYTHON=python3.13 \
    UV_PROJECT_ENVIRONMENT=/app

# Copy code into builder
COPY . /src
WORKDIR /src/data-plane/python-bindings

# Install just the slim-bindings-examples package
RUN uv sync --frozen --no-dev --no-editable --package slim-bindings-examples

FROM python:3.13-bookworm AS slim-bindings-examples

# Copy venv from builder with just the dependencies we need + our package
COPY --from=builder --chown=app:app /app /app

ENTRYPOINT ["/app/bin/examples"]
