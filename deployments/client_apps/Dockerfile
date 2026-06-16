# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

FROM python:3.12-slim

# Pinned to a published slim-bindings release; bindings are maintained in
# https://github.com/agntcy/slim-bindings and published to PyPI as slim-bindings.
ARG SLIM_BINDINGS_VERSION=1.4.1
RUN pip install --no-cache-dir "slim-bindings[examples]==${SLIM_BINDINGS_VERSION}"

# Run example CLIs directly (e.g. Kubernetes: set command/args to slim-bindings-p2p [...])
CMD ["slim-bindings-group", "--help"]
