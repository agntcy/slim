# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime
import logging


from mcp import ClientSession

from agp_mcp import AGPClient

# Configure logging
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

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
        async with agp_client.to_mcp_session() as mcp_session:
            logger.info("initialize session")
            await mcp_session.initialize()

            # Test tool listing
            tools = await mcp_session.list_tools()
            assert tools is not None, "Failed to list tools"
            logger.info(f"Successfully retrieved tools: {tools}")

asyncio.run(main())