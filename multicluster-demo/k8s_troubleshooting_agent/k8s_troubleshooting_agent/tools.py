"""Tools for the Kubernetes troubleshooting agent."""

import json
import logging
from typing import Annotated

from pydantic import Field

from k8s_troubleshooting_agent.mcp_client import K8sMCPClient

logger = logging.getLogger(__name__)

# Global MCP client instance (will be initialized in main.py)
mcp_client: K8sMCPClient | None = None


def set_mcp_client(client: K8sMCPClient) -> None:
    """Set the global MCP client instance."""
    global mcp_client
    mcp_client = client


async def list_available_mcp_tools() -> str:
    """List all available MCP tools from the Kubernetes MCP server.

    Use this to discover what tools are available for querying the cluster.
    Always call this first to know what operations you can perform.
    
    Returns a JSON array of tools, each with:
    - name: The tool name to use with call_mcp_tool
    - description: What the tool does
    - input_schema: JSON schema describing required/optional parameters
    """
    if not mcp_client:
        return "Error: MCP client not initialized"

    try:
        tools = await mcp_client.list_available_tools()
        return json.dumps(tools, indent=2)
    except Exception as e:
        logger.error(f"Error listing MCP tools: {e}")
        return f"Error listing MCP tools: {e}"


async def call_mcp_tool(
    tool_name: Annotated[str, Field(description="Name of the MCP tool to call")],
    arguments: Annotated[dict, Field(description="Tool arguments as a dictionary")] = None,
) -> str:
    """Call any available MCP tool to query the Kubernetes cluster.

    Use this to perform any operation discovered via list_available_mcp_tools.
    Always call list_available_mcp_tools first to see what tools exist and what parameters they accept.
    
    Args:
        tool_name: Name of the tool to call
        arguments: Dictionary of arguments (optional, defaults to empty dict)
    """
    if not mcp_client:
        return "Error: MCP client not initialized"

    if arguments is None:
        arguments = {}
        
    logger.debug(f"Calling MCP tool: {tool_name} with args: {arguments}")

    try:
        result = await mcp_client.call_tool(tool_name, arguments)
        logger.debug(f"MCP tool {tool_name} returned successfully")
        return result
    except Exception as e:
        logger.error(f"Error calling MCP tool {tool_name}: {e}", exc_info=True)
        return f"Error calling MCP tool {tool_name}: {e}"

