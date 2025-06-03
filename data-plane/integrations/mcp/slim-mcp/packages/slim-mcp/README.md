# SLIM-MCP Integration

Leverage SLIM as a transport mechanism for MCP, enabling efficient load balancing
and dynamic discovery across MCP servers.

## Installation

```bash
pip install slim-mcp
```

## Overview

SLIM-MCP provides a seamless integration between SLIM (Secure Low-Latency
Interactive Messaging) and MCP (Model Context Protocol), allowing you to:

- Create MCP servers that can be discovered and accessed through SLIM
- Connect MCP clients to servers using SLIM as the transport layer
- Handle multiple concurrent sessions
- Leverage SLIM's load balancing and service discovery capabilities

## Usage

### Server Setup

```python
from slim_mcp import SLIMServer
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

# Configure and start the SLIM server
config = {
    "endpoint": "http://127.0.0.1:12345",
    "tls": {
        "insecure": True,
    },
}

async with SLIMServer(config, "org", "namespace", "server-name") as slim_server:
    # Handle incoming sessions
    async for session in slim_server:
        async with slim_server.new_streams(session) as streams:
            await app.run(
                streams[0],
                streams[1],
                app.create_initialization_options(),
            )
```

### Client Setup

```python
from slim_mcp import SLIMClient

# Configure the client
config = {
    "endpoint": "http://127.0.0.1:12345",
    "tls": {
        "insecure": True,
    },
}

async with SLIMClient(
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

- **Automatic Reconnection**: SLIM automatically handles reconnection to the server if the connection is lost
- **Concurrent Sessions**: Support for multiple concurrent sessions with proper resource management
- **TLS Support**: Built-in support for secure TLS connections
- **Dynamic Discovery**: Leverage SLIM's service discovery capabilities to find and connect to MCP servers
- **Load Balancing**: Utilize SLIM's load balancing features for optimal server distribution

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
