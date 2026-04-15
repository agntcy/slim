# a2acli Command Patterns

Use the installed binary at `~/.copilot/skills/a2acli/scripts/a2acli`.

## Discover Agents

```bash
~/.copilot/skills/a2acli/scripts/a2acli list
```

If AgentCard files are not in the current working directory, pass `--agents-dir <path>` or place them in `~/.a2aagents/`.

## Inspect An Agent

```bash
~/.copilot/skills/a2acli/scripts/a2acli get-card --agent sha256:<digest-prefix>
```

Use this before delegation when you need to confirm the agent skills, capabilities, or protocol binding.

## Send A Message

```bash
~/.copilot/skills/a2acli/scripts/a2acli send-message --agent sha256:<digest-prefix> "What issues do you see in the cluster?"
```

For asynchronous work:

```bash
~/.copilot/skills/a2acli/scripts/a2acli send-message --agent sha256:<digest-prefix> --return-immediately "Check the cluster and create a Jira issue if needed."
~/.copilot/skills/a2acli/scripts/a2acli get-task --agent sha256:<digest-prefix> <task-id>
```

## Multicluster Demo Prompt Shapes

- `Check the cluster and summarize any failing workloads in the demo namespace.`
- `Explain whether the current failures look like CrashLoopBackOff, ImagePullBackOff, a failed Job, or an unschedulable pod.`
- `Create a Jira issue for the current failure and include the affected workload names.`