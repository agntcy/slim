
import logging
from typing import Any, Optional

from google.adk.tools.base_tool import BaseTool
from google.adk.tools.base_toolset import BaseToolset
from google.adk.tools.tool_context import ToolContext
from google.adk.agents.readonly_context import ReadonlyContext
from google.genai.types import FunctionDeclaration
from mcp.types import Tool as McpBaseTool
from typing_extensions import override

from k8s_troubleshooting_agent.mcp_client import K8sMCPClient

logger = logging.getLogger(__name__)

class SLIMMcpTool(BaseTool):
    """Wraps an MCP tool to work with ADK framework using SLIM MCP client."""

    def __init__(
        self,
        mcp_tool: McpBaseTool,
        mcp_client: K8sMCPClient,
    ):
        """Initialize a SLIM MCP tool.

        Args:
            mcp_tool: The MCP tool definition from the server
            mcp_client: The K8sMCPClient instance for communication
        """
        super().__init__(
            name=mcp_tool.name,
            description=mcp_tool.description if mcp_tool.description else "",
        )
        self._mcp_tool = mcp_tool
        self._mcp_client = mcp_client

    @override
    def _get_declaration(self) -> FunctionDeclaration:
        """Gets the function declaration for the tool.

        Returns:
            FunctionDeclaration: The Gemini function declaration for the tool.
        """
        input_schema = self._mcp_tool.inputSchema
        output_schema = self._mcp_tool.outputSchema
        
        function_decl = FunctionDeclaration(
            name=self.name,
            description=self.description,
            parameters_json_schema=input_schema,
            response_json_schema=output_schema,
        )
        return function_decl

    @override
    async def run_async(
        self, *, args: dict[str, Any], tool_context: ToolContext
    ) -> Any:
        """Runs the tool with the given arguments.

        Args:
            args: The LLM-filled arguments.
            tool_context: The context of the tool.

        Returns:
            The result of running the tool.
        """
        try:
            logger.debug(f"Calling MCP tool: {self.name} with args: {args}")
            result = await self._mcp_client.call_tool(self.name, args)
            logger.debug(f"MCP tool {self.name} returned successfully")
            return result
        except Exception as e:
            logger.error(f"Error calling MCP tool {self.name}: {e}", exc_info=True)
            return {"error": f"Error calling MCP tool {self.name}: {e}"}


class SLIMMcpToolSet(BaseToolset):
    """Toolset that provides MCP tools via SLIM MCP client.
    
    This toolset automatically fetches available tools from the MCP server
    and wraps them as ADK tools.
    """

    def __init__(
        self,
        mcp_client: K8sMCPClient,
        tool_filter: Optional[list[str]] = None,
        tool_name_prefix: Optional[str] = None,
    ):
        """Initialize the SLIM MCP toolset.

        Args:
            mcp_client: The K8sMCPClient instance for communication
            tool_filter: Optional list of tool names to include (filters others out)
            tool_name_prefix: Optional prefix to add to all tool names
        """
        super().__init__(
            tool_filter=tool_filter,
            tool_name_prefix=tool_name_prefix,
        )
        self._mcp_client = mcp_client

    @override
    async def get_tools(
        self,
        readonly_context: Optional[ReadonlyContext] = None,
    ) -> list[BaseTool]:
        """Return all tools from the MCP server.

        Args:
            readonly_context: Context used to filter tools available to the agent.
                If None, all tools in the toolset are returned.

        Returns:
            List of BaseTool instances for each available MCP tool
        """
        try:
            # Fetch available tools from the MCP server
            tools_data = await self._mcp_client.list_available_tools()
            
            tools = []
            for tool_data in tools_data:
                # Reconstruct McpBaseTool from the dict
                mcp_tool = McpBaseTool(
                    # required fields
                    name=tool_data["name"],
                    inputSchema=tool_data["input_schema"],
                    # Optional fields with defaults
                    description=tool_data.get("description", ""),
                    outputSchema=tool_data.get("output_schema", {}),  
                )
                
                # Wrap as SLIMMcpTool
                slim_tool = SLIMMcpTool(
                    mcp_tool=mcp_tool,
                    mcp_client=self._mcp_client,
                )
                
                # Apply tool filter if specified
                if self._is_tool_selected(slim_tool, readonly_context):
                    tools.append(slim_tool)
            
            logger.info(f"Loaded {len(tools)} MCP tools from server")
            return tools
            
        except Exception as e:
            logger.error(f"Error fetching tools from MCP server: {e}", exc_info=True)
            return []

    async def close(self) -> None:
        """Cleanup resources (no-op for SLIM client which is managed externally)."""
        # The K8sMCPClient is managed by the main application lifecycle
        # so we don't close it here
        pass
