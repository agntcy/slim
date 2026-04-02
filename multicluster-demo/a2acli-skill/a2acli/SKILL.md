---
name: a2acli
description: Interact with A2A protocol agents — list available agents, inspect their capabilities, send messages, and retrieve task results. Use when you need to communicate with or delegate work to another A2A-compatible AI agent, query an agent's skills, or check the status of an async task.
license: Apache-2.0
compatibility: Requires the `a2acli` binary to be built and available in PATH. Build it by running `task install` from the skill directory, or `task build-skill` to place the binary directly in this skill's `scripts/` directory. AgentCard JSON files must be present in a `.a2aagents` directory (in the current working directory or the user's home directory).
metadata:
  author: tehsmash
  version: "0.1.0"
allowed-tools: Bash(a2acli:*)
---

# a2acli — A2A Protocol CLI

`a2acli` is a command-line tool for interacting with agents that expose the
[A2A (Agent-to-Agent) protocol](https://github.com/a2aproject/A2A). It supports
both standard HTTP transports (JSON-RPC, REST) and SLIM RPC for agents deployed
behind a [SLIM](https://docs.agntcy.org/slim/) messaging node.

## Setup

### 1. Build the binary

From the skill directory:

```bash
# Build and place in this skill's scripts/ directory
task build-skill

# Or install to your PATH
task install
```

### 2. Add AgentCard files

AgentCard JSON files must be placed in a `.a2aagents` directory. The CLI searches:

1. `.a2aagents/` in the **current working directory**
2. `~/.a2aagents/` in the **home directory**

You can override this with `--agents-dir <path>`.

Each agent is identified by the **SHA256 digest** of its AgentCard JSON file (e.g. `sha256:3f7a2c1b...`). You can use the full digest or any unambiguous prefix.

---

## Commands

### `a2acli list`

List all available agents.

```
a2acli list [--agents-dir <path>]
```

**Output:**
```
NAME                  DESCRIPTION                                       DIGEST
weather-agent         Provides weather forecasts for any location        sha256:3f7a2c1b...
code-review-agent     Reviews Go, Python and TypeScript code             sha256:9d4e8f2a...
```

---

### `a2acli get-card --agent <digest>`

Print the full AgentCard JSON for an agent, including all skills, capabilities, and supported interfaces.

```
a2acli get-card --agent <digest>
```

**Example:**
```
a2acli get-card --agent sha256:3f7a2c1b
```

**Output:** Pretty-printed AgentCard JSON, for example:
```json
{
  "name": "weather-agent",
  "description": "Provides weather forecasts for any location",
  "version": "1.0",
  "skills": [
    {
      "id": "get-forecast",
      "name": "Get Forecast",
      "description": "Returns a weather forecast for a given location and date range",
      "tags": ["weather", "forecast"]
    }
  ],
  "capabilities": {
    "streaming": false
  },
  ...
}
```

Use this command when you need to understand what an agent can do before sending it a message.

---

### `a2acli send-message --agent <digest> [--return-immediately] "<message>"`

Send a text message to an agent.

```
a2acli send-message --agent <digest> [--return-immediately] "<message>"
```

**Flags:**
- `--return-immediately` — Instruct the agent to return a Task immediately without waiting for completion. The Task ID and initial status are printed; no polling is performed. Without this flag the CLI polls every 2 seconds until the task reaches a terminal state.

**Example — synchronous response (agent replies with a Message):**
```
$ a2acli send-message --agent sha256:3f7a2c1b "What is the weather in London tomorrow?"
The forecast for London tomorrow is partly cloudy with a high of 18°C and a low of 11°C.
```

**Example — async response, poll until done (default):**
```
$ a2acli send-message --agent sha256:9d4e8f2a "Review the code in main.go for security issues"
Task ID: 01938f4a-1234-7abc-def0-123456789abc
Waiting for task to complete...
Status: TASK_STATE_WORKING
Status: TASK_STATE_COMPLETED

Final status: TASK_STATE_COMPLETED

Artifact 1: code-review
Found 2 potential issues:
1. SQL query on line 42 uses string concatenation — use parameterized queries instead.
2. Error from os.ReadFile on line 67 is silently ignored.
```

**Example — return immediately, retrieve result later:**
```
$ a2acli send-message --agent sha256:9d4e8f2a --return-immediately "Generate a report"
Task ID: 01938f4a-1234-7abc-def0-123456789abc
Status:  TASK_STATE_SUBMITTED
```

---

### `a2acli get-task --agent <digest> "<task-id>"`

Retrieve the current state of a task, including its status, history, and any artifacts.

```
a2acli get-task --agent <digest> "<task-id>"
```

**Example:**
```
$ a2acli get-task --agent sha256:9d4e8f2a "01938f4a-1234-7abc-def0-123456789abc"
```

**Output:** The full task as JSON:
```json
{
  "id": "01938f4a-1234-7abc-def0-123456789abc",
  "contextId": "01938f49-5678-7abc-def0-abcdef012345",
  "status": {
    "state": "TASK_STATE_COMPLETED"
  },
  "artifacts": [
    {
      "artifactId": "01938f4b-...",
      "name": "code-review",
      "parts": [
        {
          "text": "Found 2 potential issues:\n1. ..."
        }
      ]
    }
  ]
}
```

---

## Global Flags

| Flag | Description |
|------|-------------|
| `--agents-dir <path>` | Override the agents directory (default: `.a2aagents` in CWD, then `~/.a2aagents`) |

---

## Common Workflows

### Discover and query an agent

```bash
# 1. List available agents to find the one you need
a2acli list

# 2. Inspect its capabilities and skills
a2acli get-card --agent sha256:3f7a2c1b

# 3. Send a message and wait for the result
a2acli send-message --agent sha256:3f7a2c1b "Hello, what can you do?"
```

### Fire-and-forget a long-running task

```bash
# Submit the task and get the ID immediately
a2acli send-message --agent sha256:9d4e8f2a --return-immediately "Analyze the sales data for Q1 2025"
# Task ID: 01938f4a-1234-7abc-def0-123456789abc
# Status:  TASK_STATE_SUBMITTED

# Check back later
a2acli get-task --agent sha256:9d4e8f2a "01938f4a-1234-7abc-def0-123456789abc"
```

### Use a non-default agents directory

```bash
a2acli --agents-dir /path/to/my/agents list
a2acli --agents-dir /path/to/my/agents send-message --agent sha256:3f7a2c1b "Hello"
```

---

## Task States

| State | Meaning |
|-------|---------|
| `TASK_STATE_SUBMITTED` | Task has been received, awaiting execution |
| `TASK_STATE_WORKING` | Agent is actively working on the task |
| `TASK_STATE_COMPLETED` | Task finished successfully |
| `TASK_STATE_FAILED` | Task failed due to an error |
| `TASK_STATE_CANCELED` | Task was canceled |
| `TASK_STATE_REJECTED` | Task was rejected by the agent |
| `TASK_STATE_INPUT_REQUIRED` | Agent needs more input (send a follow-up message) |
| `TASK_STATE_AUTH_REQUIRED` | Agent requires authentication |
