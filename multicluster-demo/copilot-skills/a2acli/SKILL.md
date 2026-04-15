---
name: a2acli
description: 'Use a2acli to list A2A agents, inspect AgentCards, send messages, and fetch task results. Use when the multicluster demo operator needs to interact with the cloud Kubernetes troubleshooting agent from a local laptop, especially with local SPIRE-backed SLIM auth.'
argument-hint: 'list agents | inspect card | send message | get task'
user-invocable: true
---

# a2acli

Use this skill when you need to:

- discover A2A agents from `.a2aagents/` or `~/.a2aagents/`
- inspect an agent card before you delegate work
- send a message to the multicluster-demo troubleshooting agent from the IT-OPS laptop
- fetch a task result after sending an asynchronous request

## Before You Use It

1. From `slim/multicluster-demo/a2acli-skill`, run `task install-copilot-skill`.
2. Confirm the installed binary exists at `~/.copilot/skills/a2acli/scripts/a2acli`.
3. Ensure AgentCard JSON files exist in `.a2aagents/` in the current workspace or in `~/.a2aagents/`.
4. Ensure `.a2acli.yaml` or `~/.a2acli.yaml` points at the correct SLIM endpoint and, for the multicluster demo, the local SPIRE Workload API socket.

## Common Tasks

1. Discover agents:
   `~/.copilot/skills/a2acli/scripts/a2acli list`
2. Inspect an agent card:
   `~/.copilot/skills/a2acli/scripts/a2acli get-card --agent <digest>`
3. Send a message:
   `~/.copilot/skills/a2acli/scripts/a2acli send-message --agent <digest> "<message>"`
4. Submit an asynchronous task and fetch it later:
   `~/.copilot/skills/a2acli/scripts/a2acli send-message --agent <digest> --return-immediately "<message>"`
   `~/.copilot/skills/a2acli/scripts/a2acli get-task --agent <digest> <task-id>`

## Multicluster Demo Workflow

1. The adversary injects standard Kubernetes issues into the demo cluster.
2. The existing troubleshooting agent inspects the cluster and can create Jira issues when asked.
3. The IT-OPS operator uses local `a2acli` to query the cloud agent, ask follow-up questions, or pull task results.

Keep secrets and environment-specific values in local config files such as `.a2acli.yaml` or the multicluster-demo `.env.local` workflow. Do not embed secrets directly into skill instructions.

See [command patterns](./references/command-patterns.md) and [multicluster demo workflow](./references/multicluster-demo.md).