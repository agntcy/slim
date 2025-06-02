# SLIM-MCP proxy
This proxy enables connecting existing MCP servers that use the SSE transport to the SLIM network. The proxy is capable of receiving messages from an application running on top of SLIM and forwarding them to the SSE server (and vice versa).

## How to run the code
You can use the commands provided in the Taskfile to run the client and server located in the example folder.

### Run the SLIM node
To run the SLIM node, use the following command:
```bash
task mcp-proxy:run-slim
```
### Run the SLIM-MCP proxy
To run the SLIM-MCP proxy, use the following command:
```bash
task mcp-proxy:run-mcp-proxy
```
### Run the MCP Server
To run the MCP server example, use the following command:
```bash
task mcp-proxy:test:run-mcp-server
```
The MPC server will start listening on port 8000
### Run the MCP Client
To run the MCP client example, use the following command:
```bash
task mcp-proxy:test:run-mcp-client
```
The client uses the SLIM transport, so it will connect to the SLIM node and communicate with the MCP server through the proxy.
