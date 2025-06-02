# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime
import logging
import time

from mcp import ClientSession, types
from mcp.types import AnyUrl

from slim_mcp import SLIMClient

# Configure logging
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

subscription = False
unsubscription = False

async def logging_callback_fn(
    params: types.LoggingMessageNotificationParams,
) -> None:
    logger.info(f"Received Server Log Notification {params}")
    if params.data == "subscribe_resource":
        global subscription
        subscription = True
    if params.data == "unsubscribe_resource":
        global unsubscription
        unsubscription = True

async def main():
    org = "org"
    ns = "mcp"
    mcp_server = "proxy"

    # create mcp server with SLIM transport
    config = {
        "endpoint": "http://127.0.0.1:46357",
        "tls": {
            "insecure": True,
        },
    }

    async with (
        SLIMClient(config, org, ns, "client1", org, ns, mcp_server) as slim_client,
    ):
        async with slim_client.to_mcp_session(logging_callback=logging_callback_fn) as mcp_session:
            logger.info("initialize session")
            await mcp_session.initialize()

            # Test tool listing
            tools = await mcp_session.list_tools()
            if tools is None:
                logger.error("Failed to list tools")
                return
            logger.info(f"Successfully retrieved tools: {tools}")

            # Test use fetch tool
            res = await mcp_session.call_tool("fetch", {"url": "https://example.com"})
            if res is None:
                logger.error("Failed to use the fetch tool")
                return
            logger.info(f"Successfully used tool: {res}")

            # List available resources
            resources = await mcp_session.list_resources()
            if resources is None:
                logger.error("Failed to use list resources")
                return
            logger.info(f"Successfully list resources: {resources}")

            # Subscribe for a resource
            await mcp_session.subscribe_resource(AnyUrl("file:///greeting.txt"))

            time.sleep(1)
            if subscription == False:
                logger.error("Failed to subscribe for the resource")
                return
            logger.info("Successfully processed subscription")

            # read a specific resource
            resource = await mcp_session.read_resource(AnyUrl("file:///greeting.txt"))
            if resource is None:
                logger.error("Failed to read a resource")
                return
            logger.info(f"Successfully used resource: {resource}")

            # Unubscribe for a resource
            await mcp_session.unsubscribe_resource(AnyUrl("file:///greeting.txt"))

            time.sleep(1)
            if unsubscription == False:
                logger.error("Failed to unsubscribe for the resource")
                return
            logger.info("Successfully processed unsubscription")

            # List available prompts
            prompts = await mcp_session.list_prompts()
            if prompts is None:
                logger.error("Failed to list the prompts")
                return
            logger.info(f"Successfully list prompts: {prompts}")

            # Get the prompt with arguments
            prompt = await mcp_session.get_prompt(
                "simple",
                {
                    "context": "User is a software developer",
                    "topic": "Python async programming",
                },
            )
            if prompt is None:
                logger.error("Failed to get the prompt")
                return
            logger.info(f"Successfully got prompt: {prompt}")

if __name__ == "__main__":
    asyncio.run(main())
