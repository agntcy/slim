# MCP Time Server Example

This example demonstrates how to create a Model Context Protocol (MCP) server
that provides time-related functionality for Large Language Models (LLMs).
The server offers tools for getting current time in different timezones and
converting times between timezones.

## Features

- Get current time in any IANA timezone
- Convert times between different timezones
- Support for daylight saving time (DST)
- Time difference calculations between timezones
- ISO 8601 formatted datetime outputs

## Prerequisites

- Python 3.10 or higher
- MCP library (version 1.6.0 or higher)
- SLIM-MCP library (version 0.1.0 or higher)
- Click library (version 8.1.8 or higher)

## Usage

The server can be started using the following command:

```bash
uv run --package mcp-server-time mcp-server-time --local-timezone Europe/London
```

### Configuration Options

The server can be configured using environment variables or command-line arguments. The following table shows all available options:

| Option         | Command-line Argument | Environment Variable             | Default Value                                                       |
| -------------- | --------------------- | -------------------------------- | ------------------------------------------------------------------- |
| Local Timezone | `--local-timezone`    | `MCP_TIME_SERVER_LOCAL_TIMEZONE` | System timezone                                                     |
| Organization   | `--organization`      | `MCP_TIME_SERVER_ORGANIZATION`   | "org"                                                               |
| Namespace      | `--namespace`         | `MCP_TIME_SERVER_NAMESPACE`      | "ns"                                                                |
| Server Name    | `--mcp-server`        | `MCP_TIME_SERVER_MCP_SERVER`     | "time-server"                                                       |
| Server Config  | `--config`            | `MCP_TIME_SERVER_CONFIG`         | `{"endpoint": "http://127.0.0.1:46357", "tls": {"insecure": true}}` |

Example with custom configuration:

```bash
uv run --package mcp-server-time mcp-server-time \
  --local-timezone "America/New_York" \
  --organization "myorg" \
  --namespace "mytime"
```

## Available Tools

The server provides two main tools:

1. `get_current_time`: Get the current time in a specified timezone

   - Input: IANA timezone name
   - Output: Current time with timezone information and DST status

2. `convert_time`: Convert time between timezones
   - Input: Source timezone, time (HH:MM format), and target timezone
   - Output: Converted time with timezone information and time difference

## License

This project is licensed under the Apache-2.0 License.
