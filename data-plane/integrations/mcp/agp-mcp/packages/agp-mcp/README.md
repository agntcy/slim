# AGP-MCP Integration

Leverage AGP as a transport mechanism for MCP, enabling efficient load balancing
and dynamic discovery across MCP servers.

## Installation

```bash
pip install agp-mcp
```

## Overview

AGP-MCP provides a seamless integration between AGP (Agent Gateway Protocol)
and MCP (Model Context Protocol), allowing you to:

- Create MCP servers that can be discovered and accessed through AGP
- Connect MCP clients to servers using AGP as the transport layer
- Handle multiple concurrent sessions
- Leverage AGP's load balancing and service discovery capabilities

## Usage

### Server Setup

```python
from agp_mcp import AGPServer
import mcp.types as types
from mcp.server.lowlevel import Server

# Create an MCP server application
app = Server("example-server")

# Define your tools
example_tool = types.Tool(
    name="example",
    description="An example tool",
    inputSchema={
        "type": "object",
        "required": ["url"],
        "properties": {
            "url": {
                "type": "string",
                "description": "example URL input parameter",
            }
        },
    },
)

@app.list_tools()
async def list_tools() -> list[types.Tool]:
    return [example_tool]

# Configure and start the AGP server
config = {
    "endpoint": "http://127.0.0.1:12345",
    "tls": {
        "insecure": True,
    },
}

async with AGPServer(config, "org", "namespace", "server-name") as agp_server:
    # Handle incoming sessions
    async for session in agp_server:
        async with agp_server.new_streams(session) as streams:
            await app.run(
                streams[0],
                streams[1],
                app.create_initialization_options(),
            )
```

### Client Setup

```python
from agp_mcp import AGPClient

# Configure the client
config = {
    "endpoint": "http://127.0.0.1:12345",
    "tls": {
        "insecure": True,
    },
}

async with AGPClient(
    config,
    "org",
    "namespace",
    "client-id",
    "org",
    "namespace",
    "server-name"
) as client:
    async with client.to_mcp_session() as mcp_session:
        # Initialize the session
        await mcp_session.initialize()

        # List available tools
        tools = await mcp_session.list_tools()
        print(f"Available tools: {tools}")
```

## Features

- **Automatic Reconnection**: AGP automatically handles reconnection to the server if the connection is lost
- **Concurrent Sessions**: Support for multiple concurrent sessions with proper resource management
- **TLS Support**: Built-in support for secure TLS connections
- **Dynamic Discovery**: Leverage AGP's service discovery capabilities to find and connect to MCP servers
- **Load Balancing**: Utilize AGP's load balancing features for optimal server distribution

## Configuration

The configuration object supports the following options:

```python
config = {
    "endpoint": "http://127.0.0.1:12345",  # Server endpoint
    "tls": {
        "insecure": True,  # Set to False for production
        # Add other TLS options as needed
    },
}
```

## Error Handling

The library provides comprehensive error handling and logging. All operations
are wrapped in try-except blocks to ensure proper cleanup of resources.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

Apache-2.0
