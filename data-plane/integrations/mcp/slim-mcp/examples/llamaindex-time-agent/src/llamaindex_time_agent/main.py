# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging

import click
from dotenv import load_dotenv
from llama_index.core.agent.workflow import ReActAgent
from llama_index.llms.azure_openai import AzureOpenAI
from llama_index.llms.ollama import Ollama
from llama_index.tools.mcp import McpToolSpec

from slim_mcp import SLIMClient

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

# load .env file
load_dotenv()


async def amain(
    llm_type, llm_endpoint, llm_key, organization, namespace, mcp_server, city, config
):
    if llm_type == "azure":
        kwargs = {
            "engine": "gpt-4o-mini",
            "model": "gpt-4o-mini",
            "is_chat_model": True,
            "azure_endpoint": llm_endpoint,
            "api_key": llm_key,
            "api_version": "2024-08-01-preview",
        }
        llm = AzureOpenAI(**kwargs)
    elif llm_type == "ollama":
        kwargs = {
            "model": "llama3.2",
        }
        llm = Ollama(**kwargs)
    else:
        raise Exception("LLM type must be azure or ollama")

    logger.info("Starting SLIM client")
    async with SLIMClient(
        config,
        "org",
        "ns",
        "time-agent",
        organization,
        namespace,
        mcp_server,
    ) as client1:
        async with client1.to_mcp_session() as mcp_session:
            logger.info("Creating MCP tool spec")

            await mcp_session.initialize()

            mcp_tool_spec = McpToolSpec(
                client=mcp_session,
            )

            tools = await mcp_tool_spec.to_tool_list_async()

            agent = ReActAgent(llm=llm, tools=tools)

            response = await agent.run(
                user_msg=f"What is the current time in {city}?",
            )

            print(response)

            await mcp_session.close()


class DictParamType(click.ParamType):
    name = "dict"

    def convert(self, value, param, ctx):
        import json

        if isinstance(value, dict):
            return value  # Already a dict (for default value)
        try:
            return json.loads(value)
        except json.JSONDecodeError:
            self.fail(f"{value} is not valid JSON", param, ctx)


@click.command(context_settings={"auto_envvar_prefix": "TIME_AGENT"})
@click.option("--llm-type", default="azure")
@click.option("--llm-endpoint", default=None)
@click.option("--llm-key", default=None)
@click.option("--mcp-server-organization", default="org")
@click.option("--mcp-server-namespace", default="ns")
@click.option("--mcp-server-name", default="time-server")
@click.option("--city", default="New York")
@click.option(
    "--config",
    default={
        "endpoint": "http://127.0.0.1:46357",
        "tls": {
            "insecure": True,
        },
    },
    type=DictParamType(),
)
def main(
    llm_type,
    llm_endpoint,
    llm_key,
    mcp_server_organization,
    mcp_server_namespace,
    mcp_server_name,
    city,
    config,
):
    try:
        asyncio.run(
            amain(
                llm_type,
                llm_endpoint,
                llm_key,
                mcp_server_organization,
                mcp_server_namespace,
                mcp_server_name,
                city,
                config,
            )
        )
    except KeyboardInterrupt:
        logger.info("Keyboard interrupt")
    except Exception as e:
        logger.error(f"Error: {e}")
        raise e
