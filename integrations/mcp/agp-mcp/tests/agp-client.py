# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime

from mcp import ClientSession

from agp_mcp import AGPClient


async def main():
    org = "cisco"
    ns = "mcp"
    mcp_server = "proxy"

    # create mcp server with AGP transport
    config = {
        "endpoint": "http://127.0.0.1:46357",
        "tls": {
            "insecure": True,
        },
    }

    async with (
        AGPClient(config, org, ns, "client1", org, ns, mcp_server) as agp_client,
    ):
        async with agp_client.new_session() as streams:
            async with ClientSession(streams[0], streams[1]) as mcp_session:
                try:
                    await mcp_session.initialize()

                    print(
                        f"Client session initialized at {datetime.datetime.now().isoformat()}"
                    )

                    tools = await mcp_session.list_tools()
                    assert tools is not None
                    print(tools)
                except Exception as e:
                    print(f"Error initializing session: {e}")
                    raise e

asyncio.run(main())