# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""
MCP Time Server - A server implementation for time and timezone conversion functionality.
This module provides tools for getting current time in different timezones and converting times between timezones.
"""

import asyncio
import json
import logging
from collections.abc import Sequence
from datetime import datetime, timedelta
from enum import Enum
from zoneinfo import ZoneInfo

import click
from mcp import types
from mcp.server.lowlevel import Server
from mcp.shared.exceptions import McpError
from pydantic import BaseModel

from slim_mcp import SLIMServer, init_tracing

# Configure logging
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


class TimeTools(str, Enum):
    """Enumeration of available time-related tools."""

    GET_CURRENT_TIME = "get_current_time"  # Tool to get current time in a timezone
    CONVERT_TIME = "convert_time"  # Tool to convert time between timezones


class TimeResult(BaseModel):
    """Model representing a time result with timezone information."""

    timezone: str  # IANA timezone name
    datetime: str  # ISO formatted datetime string
    is_dst: bool  # Whether the timezone is in daylight saving time


class TimeConversionResult(BaseModel):
    """Model representing the result of a time conversion between timezones."""

    source: TimeResult  # Source timezone information
    target: TimeResult  # Target timezone information
    time_difference: str  # String representation of time difference (e.g., "+2.0h")


class TimeConversionInput(BaseModel):
    """Model for time conversion input parameters."""

    source_tz: str  # Source timezone
    time: str  # Time to convert in HH:MM format
    target_tz_list: list[str]  # List of target timezones


def get_local_tz(local_tz_override: str | None = None) -> ZoneInfo:
    """
    Get the local timezone information.

    Args:
        local_tz_override: Optional timezone override string

    Returns:
        ZoneInfo: The local timezone information

    Raises:
        McpError: If timezone cannot be determined
    """
    if local_tz_override:
        return ZoneInfo(local_tz_override)

    # Get local timezone from datetime.now()
    tzinfo = datetime.now().astimezone(tz=None).tzinfo
    if tzinfo is not None:
        return ZoneInfo(str(tzinfo))
    raise McpError(
        types.ErrorData(
            code=types.INTERNAL_ERROR,
            message="Could not determine local timezone - tzinfo is None",
        )
    )


def get_zoneinfo(timezone_name: str) -> ZoneInfo:
    """
    Get ZoneInfo object for a given timezone name.

    Args:
        timezone_name: IANA timezone name

    Returns:
        ZoneInfo: The timezone information

    Raises:
        McpError: If timezone is invalid
    """
    try:
        return ZoneInfo(timezone_name)
    except Exception as e:
        raise McpError(
            types.ErrorData(
                code=types.INTERNAL_ERROR,
                message=f"Invalid timezone: {str(e)}",
            )
        )


class TimeServer:
    """Core time server implementation providing time-related functionality."""

    def get_current_time(self, timezone_name: str) -> TimeResult:
        """
        Get current time in specified timezone.

        Args:
            timezone_name: IANA timezone name

        Returns:
            TimeResult: Current time information in the specified timezone
        """
        timezone = get_zoneinfo(timezone_name)
        current_time = datetime.now(timezone)

        return TimeResult(
            timezone=timezone_name,
            datetime=current_time.isoformat(timespec="seconds"),
            is_dst=bool(current_time.dst()),
        )

    def convert_time(
        self, source_tz: str, time_str: str, target_tz: str
    ) -> TimeConversionResult:
        """
        Convert time between timezones.

        Args:
            source_tz: Source timezone name
            time_str: Time to convert in HH:MM format
            target_tz: Target timezone name

        Returns:
            TimeConversionResult: Converted time information

        Raises:
            ValueError: If time format is invalid
        """
        source_timezone = get_zoneinfo(source_tz)
        target_timezone = get_zoneinfo(target_tz)

        try:
            parsed_time = datetime.strptime(time_str, "%H:%M").time()
        except ValueError:
            raise ValueError("Invalid time format. Expected HH:MM [24-hour format]")

        # Create a datetime object for today with the specified time
        now = datetime.now(source_timezone)
        source_time = datetime(
            now.year,
            now.month,
            now.day,
            parsed_time.hour,
            parsed_time.minute,
            tzinfo=source_timezone,
        )

        # Convert to target timezone
        target_time = source_time.astimezone(target_timezone)

        # Calculate time difference between timezones
        source_offset = source_time.utcoffset() or timedelta()
        target_offset = target_time.utcoffset() or timedelta()
        hours_difference = (target_offset - source_offset).total_seconds() / 3600

        # Format time difference string
        if hours_difference.is_integer():
            time_diff_str = f"{hours_difference:+.1f}h"
        else:
            # For fractional hours like Nepal's UTC+5:45
            time_diff_str = f"{hours_difference:+.2f}".rstrip("0").rstrip(".") + "h"

        return TimeConversionResult(
            source=TimeResult(
                timezone=source_tz,
                datetime=source_time.isoformat(timespec="seconds"),
                is_dst=bool(source_time.dst()),
            ),
            target=TimeResult(
                timezone=target_tz,
                datetime=target_time.isoformat(timespec="seconds"),
                is_dst=bool(target_time.dst()),
            ),
            time_difference=time_diff_str,
        )


class TimeServerApp:
    """Main application class for the MCP Time Server."""

    def __init__(self, local_timezone: str | None = None):
        """
        Initialize the Time Server application.

        Args:
            local_timezone: Optional override for local timezone
        """
        self.app: Server = Server("mcp-time")
        self.time_server = TimeServer()
        self.local_tz = str(get_local_tz(local_timezone))
        self._setup_tools()

    def _setup_tools(self):
        """Setup tool definitions and handlers for the MCP server."""

        @self.app.list_tools()
        async def list_tools() -> list[types.Tool]:
            """
            List available time tools.

            Returns:
                list[types.Tool]: List of available time-related tools
            """
            return [
                types.Tool(
                    name=TimeTools.GET_CURRENT_TIME.value,
                    description="Get current time in a specific timezones",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "timezone": {
                                "type": "string",
                                "description": f"IANA timezone name (e.g., 'America/New_York', 'Europe/London'). Use '{self.local_tz}' as local timezone if no timezone provided by the user.",
                            }
                        },
                        "required": ["timezone"],
                    },
                ),
                types.Tool(
                    name=TimeTools.CONVERT_TIME.value,
                    description="Convert time between timezones",
                    inputSchema={
                        "type": "object",
                        "properties": {
                            "source_timezone": {
                                "type": "string",
                                "description": f"Source IANA timezone name (e.g., 'America/New_York', 'Europe/London'). Use '{self.local_tz}' as local timezone if no source timezone provided by the user.",
                            },
                            "time": {
                                "type": "string",
                                "description": "Time to convert in 24-hour format (HH:MM)",
                            },
                            "target_timezone": {
                                "type": "string",
                                "description": f"Target IANA timezone name (e.g., 'Asia/Tokyo', 'America/San_Francisco'). Use '{self.local_tz}' as local timezone if no target timezone provided by the user.",
                            },
                        },
                        "required": ["source_timezone", "time", "target_timezone"],
                    },
                ),
            ]

        @self.app.call_tool()
        async def call_tool(
            name: str, arguments: dict
        ) -> Sequence[types.TextContent | types.ImageContent | types.EmbeddedResource]:
            """
            Handle tool calls for time queries.

            Args:
                name: Name of the tool to call
                arguments: Dictionary of tool arguments

            Returns:
                Sequence of content types containing the tool response

            Raises:
                ValueError: If tool name is unknown or arguments are invalid
            """

            result: TimeResult | TimeConversionResult

            try:
                match name:
                    case TimeTools.GET_CURRENT_TIME.value:
                        timezone = arguments.get("timezone")
                        if not timezone:
                            raise ValueError("Missing required argument: timezone")
                        result = self.time_server.get_current_time(timezone)

                    case TimeTools.CONVERT_TIME.value:
                        if not all(
                            k in arguments
                            for k in ["source_timezone", "time", "target_timezone"]
                        ):
                            raise ValueError("Missing required arguments")
                        result = self.time_server.convert_time(
                            arguments["source_timezone"],
                            arguments["time"],
                            arguments["target_timezone"],
                        )
                    case _:
                        raise ValueError(f"Unknown tool: {name}")

                return [
                    types.TextContent(
                        type="text", text=json.dumps(result.model_dump(), indent=2)
                    )
                ]

            except Exception as e:
                raise ValueError(f"Error processing mcp-server-time query: {str(e)}")

    async def handle_session(self, session, slim_server, tasks):
        """
        Handle a single session with proper error handling and logging.

        Args:
            session: The session to handle
            slim_server: The SLIM server instance
            tasks: Set of active tasks
        """
        try:
            async with slim_server.new_streams(session) as streams:
                logger.info(
                    f"new session started - session_id: {session.id}, active_sessions: {len(tasks)}"
                )
                await self.app.run(
                    streams[0],
                    streams[1],
                    self.app.create_initialization_options(),
                )
                logger.info(
                    f"session {session.id} ended - active_sessions: {len(tasks)}"
                )
        except Exception:
            logger.error(
                f"Error handling session {session.id}",
                extra={"session_id": session.id},
                exc_info=True,
            )
            raise


async def cleanup_tasks(tasks):
    """
    Clean up all tasks and wait for their completion.

    Args:
        tasks: Set of tasks to clean up
    """
    for task in tasks:
        if not task.done():
            task.cancel()

    if tasks:
        try:
            await asyncio.gather(*tasks, return_exceptions=True)
        except Exception:
            logger.error("Error during task cleanup", exc_info=True)
            raise


async def serve_slim(
    local_timezone: str | None = None,
    organization: str = "org",
    namespace: str = "ns",
    mcp_server: str = "time-server",
    config: dict = {},
) -> None:
    """
    Main server function that initializes and runs the time server using SLIM transport.

    Args:
        local_timezone: Optional override for local timezone
        organization: Organization name
        namespace: Namespace name
        mcp_server: MCP server name
        config: Server configuration dictionary
    """
    await init_tracing({"log_level": "info"})
    time_app = TimeServerApp(local_timezone)
    tasks: set[asyncio.Task] = set()

    async with SLIMServer(config, organization, namespace, mcp_server) as slim_server:
        try:
            async for new_session in slim_server:
                task = asyncio.create_task(
                    time_app.handle_session(new_session, slim_server, tasks)
                )
                tasks.add(task)
        except Exception:
            logger.error("Error in session handler", exc_info=True)
            raise
        finally:
            await cleanup_tasks(tasks)
            logger.info("Server stopped")


def serve_sse(
    local_timezone: str | None = None,
    port: int = 8000,
) -> None:
    """
    Main server function that initializes and runs the time server using SSE transport.

    Args:
        local_timezone: Optional override for local timezone
        port: Server listening port
    """
    time_app = TimeServerApp(local_timezone)

    from mcp.server.sse import SseServerTransport
    from starlette.applications import Starlette
    from starlette.responses import Response
    from starlette.routing import Mount, Route

    sse = SseServerTransport("/messages/")

    async def handle_sse(request):
        async with sse.connect_sse(
            request.scope, request.receive, request._send
        ) as streams:
            await time_app.app.run(
                streams[0], streams[1], time_app.app.create_initialization_options()
            )
        return Response()

    starlette_app = Starlette(
        debug=True,
        routes=[
            Route("/sse", endpoint=handle_sse, methods=["GET"]),
            Mount("/messages/", app=sse.handle_post_message),
        ],
    )

    import uvicorn

    uvicorn.run(starlette_app, host="0.0.0.0", port=port)


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


@click.command(context_settings={"auto_envvar_prefix": "MCP_TIME_SERVER"})
@click.option(
    "--local-timezone", type=str, help="Override local timezone", default=None
)
@click.option("--transport", default="slim", help="transport option: slim or sse")
@click.option(
    "--port",
    default="8000",
    type=int,
    help="listening port, used only with sse transport",
)
@click.option(
    "--organization",
    default="org",
    help="server organization, used only with slim transport",
)
@click.option(
    "--namespace", default="ns", help="server namespace, used only with slim transport"
)
@click.option(
    "--mcp-server",
    default="time-server",
    help="server name, used only with slim transport",
)
@click.option(
    "--config",
    default={
        "endpoint": "http://127.0.0.1:46357",
        "tls": {
            "insecure": True,
        },
    },
    type=DictParamType(),
    help="slim server configuration, used only with slim transport",
)
def main(local_timezone, transport, port, organization, namespace, mcp_server, config):
    """
    MCP Time Server - Time and timezone conversion functionality for MCP.
    """

    if transport == "slim":
        import asyncio

        asyncio.run(
            serve_slim(local_timezone, organization, namespace, mcp_server, config)
        )
    else:
        serve_sse(local_timezone, port)
