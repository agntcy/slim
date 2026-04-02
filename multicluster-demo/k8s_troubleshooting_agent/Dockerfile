FROM python:3.11-slim

COPY --from=ghcr.io/astral-sh/uv:latest /uv /usr/local/bin/uv

WORKDIR /app

COPY pyproject.toml uv.lock ./
RUN uv sync --frozen --no-dev --no-install-project

COPY k8s_troubleshooting_agent/ ./k8s_troubleshooting_agent/
COPY main.py ./
RUN uv sync --frozen --no-dev

CMD ["uv", "run", "python", "main.py"]
