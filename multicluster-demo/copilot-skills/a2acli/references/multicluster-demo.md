# Multicluster Demo Workflow

The multicluster demo has three distinct roles:

1. An adversary running in the deployed environment injects standard Kubernetes issues.
2. The `k8s_troubleshooting_agent` inspects the cluster through MCP tools and can create Jira issues when asked.
3. The IT-OPS operator uses local `a2acli`, optionally through the local Copilot skill, to interact with the cloud agent.

## Local Operator Prerequisites

1. From `slim/multicluster-demo`, run `task run` to start the local SPIRE agent.
2. Run `task register` so the local `a2acli` binary can obtain an identity from the Workload API.
3. Run `task a2acli-config` to render the local `.a2acli.yaml` from `.env.local`.
4. From `slim/multicluster-demo/a2acli-skill`, run `task install-copilot-skill`.

## Expected Local Files

- `.env.local` contains the local cluster and endpoint values and stays out of Git.
- `a2acli-skill/.a2acli.yaml` contains the rendered local `a2acli` runtime config.
- `~/.copilot/skills/a2acli/scripts/a2acli` is the installed binary used by the Copilot skill.

## Operator Flow

1. Use `a2acli list` to identify the troubleshooting agent digest.
2. Use `a2acli get-card --agent <digest>` if you need to inspect the agent capabilities first.
3. Send a troubleshooting request to the cloud agent.
4. If the task is asynchronous, fetch it later with `a2acli get-task`.

This keeps the operator path local while the troubleshooting logic and Jira integration remain on the deployed agent side.