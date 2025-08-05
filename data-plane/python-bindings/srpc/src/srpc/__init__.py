# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio

from .common import (
    common_options,
)
from .server import Server
from .rpc import Rpc


HELP = """
SRPC Server implementation.

"""


@common_options
def server_main(
    local: str,
    slim: dict,
    remote: str | None = None,
    enable_opentelemetry: bool = False,
    enable_mls: bool = False,
    shared_secret: str | None = None,
    jwt: str | None = None,
    bundle: str | None = None,
    audience: list[str] | None = None,
    invites: list[str] | None = None,
    message: str | None = None,
    iterations: int = 1,
    sticky: bool = False,
):
    server = Server(
        local=local,
        slim=slim,
        enable_opentelemetry=enable_opentelemetry,
        shared_secret=shared_secret,
    )

    try:
        asyncio.run(server.run())
    except KeyboardInterrupt:
        print("Client interrupted by user.")


if __name__ == "__main__":
    server_main()
