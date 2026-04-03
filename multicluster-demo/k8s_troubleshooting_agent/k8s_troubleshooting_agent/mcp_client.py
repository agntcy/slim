"""MCP Client for accessing Kubernetes cluster information via MCP server."""

import logging
from typing import Any

import slim_bindings
from mcp import ClientSession
from slim_mcp import create_client_streams

logger = logging.getLogger(__name__)

class K8sMCPClient:
    """Client for interacting with Kubernetes MCP server through SLIM."""

    def __init__(
        self,
        local_app: Any,
        proxy_name: str,
        connection_id: str,
    ):
        """Initialize the MCP client.

        Args:
            local_app: Existing SLIM app instance
            proxy_name: MCP proxy name in the form org/ns/name
            connection_id: SLIM connection ID
        """
        self._client_app = local_app
        self.proxy_name = proxy_name
        self._connection_id = connection_id
        self._session: ClientSession | None = None
        self._destination = slim_bindings.Name.from_string(proxy_name)

    async def set_proxy_route(self) -> None:
        """Set route to MCP proxy in SLIM (must be called after __init__)."""
        if self._connection_id is not None:
            await self._client_app.set_route_async(self._destination, self._connection_id)
        logger.info(f"Route set to MCP server at {self.proxy_name}")

    async def list_available_tools(self) -> list[dict[str, Any]]:
        """List all available tools from the MCP server.

        Returns:
            List of tool definitions
        """
        async with create_client_streams(self._client_app, self._destination) as (read, write):
            async with ClientSession(read, write) as session:
                await session.initialize()
                tools = await session.list_tools()
                return [
                    {
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.inputSchema,
                    }
                    for tool in tools.tools
                ]

    async def call_tool(self, tool_name: str, arguments: dict[str, Any]) -> str:
        """Call a tool on the MCP server.

        Args:
            tool_name: Name of the tool to call
            arguments: Tool arguments

        Returns:
            Tool result as a string
        """
        async with create_client_streams(self._client_app, self._destination) as (read, write):
            async with ClientSession(read, write) as session:
                await session.initialize()
                result = await session.call_tool(tool_name, arguments)

                if result and len(result.content) > 0:
                    output = []
                    for content in result.content:
                        if hasattr(content, "text"):
                            output.append(content.text)
                    return "\n".join(output)
                return "No result returned"

