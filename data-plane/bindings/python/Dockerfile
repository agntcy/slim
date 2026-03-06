# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

FROM python:3.12-slim

# Install slim-bindings with [examples] extra (pydantic, prompt-toolkit, etc.)
RUN pip install --no-cache-dir "slim-bindings[examples]"

COPY docker-entrypoint.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["group", "--help"]
