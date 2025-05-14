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

# Install dependencies
RUN --mount=type=cache,target=/root/.cache/uv \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    uv sync --frozen --no-dev --no-install-project --no-install-workspace

# Copy code into builder
COPY . /src
WORKDIR /src

# Install just the llamaindex-time-agent package
RUN --mount=type=cache,target=/root/.cache/uv \
  uv sync --frozen --no-sources --no-dev --no-editable --package llamaindex-time-agent

FROM python:3.13-bookworm AS llamaindex-time-agent

# Copy venv from builder with just the dependencies we need + our package
COPY --from=builder --chown=app:app /app /app

ENTRYPOINT ["/app/bin/llamaindex-time-agent"]