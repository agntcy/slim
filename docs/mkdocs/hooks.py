# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""MkDocs hooks: trust store for urllib (HTTPS includes in mkdocs-include-markdown-plugin)."""
import ssl
import certifi


def on_startup(**kwargs):
    """Configure SSL context to use certifi certificates."""
    import urllib.request

    ssl_context = ssl.create_default_context(cafile=certifi.where())
    https_handler = urllib.request.HTTPSHandler(context=ssl_context)
    opener = urllib.request.build_opener(https_handler)
    urllib.request.install_opener(opener)
