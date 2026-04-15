# k8s_troubleshooting_agent

This agent is the diagnosis component of the multicluster demo. It inspects Kubernetes state through a Kubernetes MCP server and, when asked, uses an Atlassian MCP server to check for duplicate Jira issues and create a new ticket.

The agent is not responsible for generating failures and it is not the human operator client.

## Demo Role

1. The adversary injects standard Kubernetes failure scenarios into the demo cluster.
2. A periodic workflow or a human operator invokes the troubleshooting agent over A2A.
3. The agent uses MCP tools to inspect cluster state and summarize what it finds.
4. If the caller asks for Jira tracking and Atlassian MCP is available, the agent checks for duplicates and creates a Jira issue.

## Key Files

- `main.py`: wires the agent to SLIM, the Kubernetes MCP proxy, and the Atlassian MCP proxy.
- `k8s_troubleshooting_agent/agent.py`: defines the troubleshooting and Jira-ticketing instructions.
- `chart/values.yaml`: configures the SLIM connection, MCP proxy names, and optional SPIRE settings.

## Current Scope Boundary

- The agent code should stay focused on inspection and ticketing.
- The adversary is a separate deployment concern in the multicluster demo environment.
- The IT-OPS operator workflow lives in the local `a2acli` and Copilot skill setup under the parent `multicluster-demo` directory.
