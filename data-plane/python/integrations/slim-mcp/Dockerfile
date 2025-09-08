# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

FROM ghcr.io/astral-sh/uv:0.6.4-python3.13-bookworm AS builder

ENV PYTHONUNBUFFERED=1
ENV PYTHONDONTWRITEBYTECODE=1

ENV UV_LINK_MODE=copy \
    UV_COMPILE_BYTECODE=1 \
    UV_PYTHON_DOWNLOADS=never \
    UV_PYTHON=python3.13 \
    UV_PROJECT_ENVIRONMENT=/app

RUN ls -la

# Copy code into builder
COPY . /src
WORKDIR /src

# Install just the slim-mcp package
RUN --mount=type=cache,target=/root/.cache/uv \
  uv sync --no-sources --no-editable --package slim-mcp

FROM python:3.13-bookworm AS mcp-server-time

# Copy venv from builder with just the dependencies we need + our package
COPY --from=builder --chown=app:app /app /app

ENTRYPOINT ["/app/bin/mcp-server-time"]

FROM python:3.13-bookworm AS llamaindex-time-agent

# Copy venv from builder with just the dependencies we need + our package
COPY --from=builder --chown=app:app /app /app

ENTRYPOINT ["/app/bin/llamaindex-time-agent"]
