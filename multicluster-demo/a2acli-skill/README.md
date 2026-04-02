# a2acli

A CLI tool and Claude Code skill for interacting with agents that expose the [A2A (Agent-to-Agent) protocol](https://github.com/a2aproject/A2A). Supports both standard HTTP transports (JSON-RPC, REST) and [SLIM RPC](https://docs.agntcy.org/slim/) for agents deployed behind a SLIM messaging node.

## Building

Requires Go 1.24+. The SLIM bindings use CGO with a pre-built native library that must be downloaded before the first build.

```bash
# Download the slim-bindings-go native library (run once, or after upgrading slim-bindings-go)
task setup

# Build binary to bin/a2acli
task build

# Build and place binary in the skill's scripts/ directory
task build-skill

# Build, copy to scripts/, and zip the skill for distribution
task package-skill

# Install to GOPATH/bin (adds to PATH)
task install
```

`CGO_ENABLED=1` is set automatically by the Taskfile. The `setup` step is also run automatically as a dependency of `build`, `build-skill`, and `install`.

## Configuration File

`a2acli` looks for a YAML config file in these locations (first match wins):

1. Path given by `--config <path>`
2. `.a2acli.yaml` in the current working directory
3. `~/.a2acli.yaml` in the home directory

This lets credentials and SLIM settings live in the config file so they don't need to be passed as flags on every invocation. Command-line flags always take precedence over file values.

An annotated example is provided at `.a2acli.yaml.example`:

```yaml
slim:
  endpoint: "http://localhost:46357"
  local-name: "agntcy/cli/a2acli"
  secret: "my_shared_secret"        # used when spire.socket-path is not set
  # spire:
  #   socket-path: "/tmp/spire-agent/public/api.sock"
  #   target-spiffe-id: "spiffe://example.org/agent"
  #   jwt-audiences: ["my-audience"]

# agents-dir: /path/to/my/.a2aagents
```

## Agent Cards

Agents are identified by the SHA256 digest of their AgentCard JSON file. The CLI searches for cards in:

1. `.a2aagents/` in the current working directory
2. `~/.a2aagents/` in the home directory

Override with `--agents-dir <path>`.

A demo card for the k8s troubleshooting agent is included at `.a2aagents/k8s_troubleshooting_agent.json`.

### AgentCard format

#### HTTP agent

```json
{
  "name": "my-agent",
  "description": "...",
  "version": "1.0.0",
  "supportedInterfaces": [
    {
      "url": "http://my-agent.example.com",
      "protocolBinding": "jsonrpc",
      "protocolVersion": "1.0"
    }
  ],
  "capabilities": { "streaming": true },
  "defaultInputModes": ["text/plain"],
  "defaultOutputModes": ["text/plain"],
  "skills": []
}
```

#### SLIM RPC agent

The `url` field holds the agent's three-part SLIM name (`namespace/group/agent`) rather than an HTTP URL:

```json
{
  "name": "my-slim-agent",
  "description": "...",
  "version": "1.0.0",
  "supportedInterfaces": [
    {
      "url": "agntcy/demo/my_agent",
      "protocolBinding": "slimrpc",
      "protocolVersion": "0.3"
    }
  ],
  "capabilities": { "streaming": true },
  "defaultInputModes": ["text/plain"],
  "defaultOutputModes": ["text/plain"],
  "skills": []
}
```

## Commands

### `list`

List all agents in the store.

```
a2acli list
```

### `get-card`

Print the full AgentCard JSON for an agent.

```
a2acli get-card --agent <digest>
```

### `send-message`

Send a text message to an agent. If the agent responds asynchronously with a Task, use `--wait` to poll until it reaches a terminal state.

```
a2acli send-message --agent <digest> [--wait] "<message>"
```

### `get-task`

Retrieve the current state of a task by ID.

```
a2acli get-task --agent <digest> <task-id>
```

## Global Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--agents-dir` | | Path to agents directory (overrides auto-discovery) |
| `--slim-endpoint` | | SLIM node URL, e.g. `http://localhost:46357`. When set, SLIM RPC transport is enabled. |
| `--slim-local-name` | `agntcy/cli/a2acli` | Local SLIM identity (`namespace/group/name`) |
| `--slim-secret` | | Shared secret for SLIM authentication (used when `--spire-socket-path` is not set) |
| `--spire-socket-path` | | SPIRE Workload API socket path. When set, SPIRE identity auth is used instead of shared secret. |
| `--spire-target-spiffe-id` | | Target SPIFFE ID to request from SPIRE (optional) |
| `--spire-jwt-audience` | | JWT audience to request from SPIRE; may be repeated (optional) |

## SLIM Transport

When `--slim-endpoint` is provided the CLI connects to a SLIM node and registers both the v0.3 and v1.0 SLIM RPC transports. The correct transport is selected automatically based on the `protocolVersion` declared in the agent's card.

### Shared-secret auth

```bash
a2acli send-message \
  --agent sha256:6a403dbd \
  --slim-endpoint http://localhost:46357 \
  --slim-secret "my_secret" \
  "What pods are failing in my cluster?"
```

### SPIRE dynamic identity auth

When `--spire-socket-path` is set the CLI uses SPIRE's Workload API for dynamic identity instead of a shared secret:

```bash
a2acli send-message \
  --agent sha256:6a403dbd \
  --slim-endpoint http://localhost:46357 \
  --spire-socket-path /tmp/spire-agent/public/api.sock \
  --spire-target-spiffe-id spiffe://example.org/agent \
  "What pods are failing in my cluster?"
```

## Claude Code Skill

The `a2acli/` directory contains a Claude Code skill that exposes `a2acli` as a restricted tool (`Bash(a2acli:*)`). Run `task package-skill` to produce `bin/a2acli.zip` for distribution.
