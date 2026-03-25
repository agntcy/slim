# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

FROM python:3.12-slim

# Pin to match released bindings (bump with data-plane/bindings/rust/Cargo.toml)
ARG SLIM_BINDINGS_VERSION=1.3.0rc0
RUN pip install --no-cache-dir "slim-bindings[examples]==${SLIM_BINDINGS_VERSION}"

# Run example CLIs directly (e.g. Kubernetes: set command/args to slim-bindings-p2p [...])
CMD ["slim-bindings-group", "--help"]
