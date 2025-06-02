# LlamaIndex Time Agent

A llamaindex agent that uses a MCP server over SLIM for time queries and timezone conversions.
This application demonstrates how to integrate a llamaindex agent with MCP using
SLIM as transport.

## Features

- Time queries and timezone conversions using MCP server
- Support for multiple LLM backends (Azure OpenAI and Ollama)
- Interactive command-line interface

## Prerequisites

- Python 3.12 or higher
- SLIM running (default: http://127.0.0.1:46357)
- Azure OpenAI API credentials (if using Azure backend)
- Ollama installed (if using Ollama backend)

## Configuration

The application can be configured using command-line arguments or environment
variables. All options can be set using environment variables with the `TIME_AGENT_`
prefix. For example, `--llm-type` becomes `TIME_AGENT_LLM_TYPE`.

Available options:

| Option | Environment Variable | Default | Description |
|--------|---------------------|---------|-------------|
| `--llm-type` | `TIME_AGENT_LLM_TYPE` | `azure` | LLM backend type ("azure" or "ollama") |
| `--llm-endpoint` | `TIME_AGENT_LLM_ENDPOINT` | `None` | Azure OpenAI endpoint (for Azure backend) |
| `--llm-key` | `TIME_AGENT_LLM_KEY` | `None` | Azure OpenAI API key (for Azure backend) |
| `--mcp-server-organization` | `TIME_AGENT_MCP_SERVER_ORGANIZATION` | `org` | Organization name |
| `--mcp-server-namespace` | `TIME_AGENT_MCP_SERVER_NAMESPACE` | `ns` | Namespace |
| `--mcp-server-name` | `TIME_AGENT_MCP_SERVER_NAME` | `time-server` | MCP server name |
| `--config` | `TIME_AGENT_CONFIG` | `{"endpoint": "http://127.0.0.1:46357", "tls": {"insecure": true}}` | Configuration dictionary |

## Usage

With azure openai, run the application using the following command:

```bash
AZURE_OPENAI_ENDPOINT=xxx
AZURE_OPENAI_API_KEY=xxx

uv run --package llamaindex-time-agent          \
    llamaindex-time-agent                       \
    --llm-type=azure                            \
    --llm-endpoint=${AZURE_OPENAI_ENDPOINT}     \
    --llm-key=${AZURE_OPENAI_API_KEY}           \
    --city "New York"
```

With this configuration, the application will:

1. Connect to the SLIM server at http://127.0.0.1:46357
2. Use Azure OpenAI as the LLM backend
3. Initialize the time agent with the specified configuration
4. Process time-related queries

## License

Apache-2.0
