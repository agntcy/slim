import asyncio
from mcp.client.session import ClientSession
from mcp.client.stdio import StdioServerParameters, stdio_client
from mcp.client.sse import sse_client

async def main():
    stdio = False
    if stdio:
        async with stdio_client(
            StdioServerParameters(command="uv", args=["run", "mcp-simple-tool"])
        ) as (read, write):
            async with ClientSession(read, write) as session:
                await session.initialize()

            # List available tools
                tools = await session.list_tools()
                print(tools)

            # Call the fetch tool
            #result = await session.call_tool("fetch", {"url": "https://example.com"})
            #print(result)
    else:
        async with sse_client("http://127.0.0.1:8000" + "/sse", sse_read_timeout=0.5) as streams:
            async with ClientSession(*streams) as session:
                await session.initialize()

                # List available tools
                tools = await session.list_tools()
                print(tools)

            # Call the fetch tool
            #result = await session.call_tool("fetch", {"url": "https://example.com"})
            #print(result)


asyncio.run(main())


#import asyncio
#import time
#from mcp.client.session import ClientSession
#from mcp.client.stdio import StdioServerParameters, stdio_client
#from mcp.client.sse import sse_client
#
#
#async def main():
#    async with stdio_client(
#        StdioServerParameters(command="uv", args=["run", "mcp_simple_tool"])
#    ) as (read, write):
#        async with ClientSession(read, write) as session:
##    async with sse_client("http://127.0.0.1:9090" + "/sse", sse_read_timeout=0.5) as streams:
##        async with ClientSession(*streams) as session:
#            await session.initialize()
#
#            # List available tools
#            tools = await session.list_tools()
#            print(tools)
#
#            print("")
#            print("")
#            print("")
#            print("")
#
#            time.sleep(1)
#
#            # Call the fetch tool
#            result = await session.call_tool("fetch", {"url": "https://example.com"})
#            print(result)
#
#
#asyncio.run(main())
