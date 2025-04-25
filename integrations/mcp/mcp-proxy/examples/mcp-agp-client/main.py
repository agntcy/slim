# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime
import logging
import time

from mcp import ClientSession
from mcp.types import AnyUrl

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

            # Test use fetch tool
            res = await mcp_session.call_tool("fetch", {"url": "https://example.com"})
            assert res is not None, "Failed to use the fetch tool"
            logger.info(f"Successfully used tool: {res}")

            time.sleep(1)

            # List available resources
            resources = await mcp_session.list_resources()
            assert resources is not None, "Failed to use list resources"
            logger.info(f"Successfully list resources: {resources}")

            # Get a specific resource
            resource = await mcp_session.read_resource(AnyUrl("file:///greeting.txt"))
            assert resource is not None, "Failed to read a resource"
            logger.info(f"Successfully used resource: {resource}")

            time.sleep(1)

            # List available prompts
            prompts = await mcp_session.list_prompts()
            assert prompts is not None, "Failed to list the prompts"
            logger.info(f"Successfully list prompts: {prompts}")

            # Get the prompt with arguments
            prompt = await mcp_session.get_prompt(
                "simple",
                {
                    "context": "User is a software developer",
                    "topic": "Python async programming",
                },
            )
            assert prompt is not None, "Failed to get the prompt"
            logger.info(f"Successfully got prompt: {prompt}")


if __name__ == "__main__":
    asyncio.run(main())
