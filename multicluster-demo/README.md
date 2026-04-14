# Multicluster demo

This directory contains a macOS workflow for:

1. Building and running a local SPIRE agent against the EKS SPIRE server.
2. Registering the local `a2acli` binary as a SPIRE-authenticated workload.
3. Using `a2acli` to connect to a SLIM dataplane exposed by a target cluster.

## What Is In This Directory

- `Taskfile.yml`: builds the local SPIRE agent, fetches the bootstrap bundle from EKS, generates a join token, starts the agent, and registers the `a2acli` workload.
- `spire/agent.conf.tmpl`: SPIRE agent config template used to render a local runtime config.
- `a2acli-skill/`: the `a2acli` source, local agent cards, and CLI config examples.

## Prerequisites

1. macOS.
2. `go`, `task`, `kubectl`, and `git` installed.
3. Valid cloud credentials with access to the target Kubernetes cluster.
4. The following environment variables exported in your shell:

```bash
export MULTICLUSTER_DEMO_K8S_CONTEXT="<target-kubernetes-context>"
export MULTICLUSTER_DEMO_SPIRE_NAMESPACE="<spire-namespace>"
export MULTICLUSTER_DEMO_SPIRE_SERVER_POD="<spire-server-pod-name>"
export MULTICLUSTER_DEMO_SPIRE_SERVER_ADDRESS="<spire-server-hostname>"
export MULTICLUSTER_DEMO_TRUST_DOMAIN="<spiffe-trust-domain>"
export MULTICLUSTER_DEMO_SLIM_ENDPOINT="https://<slim-dataplane-host>:443"
```

## Build And Run SPIRE

Run these commands from this directory.

```bash
# Build the local SPIRE agent binary
task build

# Start the SPIRE agent
task run
```

`task run` performs the full bootstrap flow:

1. Builds `bin/spire-agent` from SPIRE source if it does not already exist.
2. Fetches the target cluster SPIRE bootstrap bundle in PEM format.
3. Generates a join token for `spiffe://<trust-domain>/macos/agent/<hostname>`.
4. Renders a local runtime config under `spire/data/agent.conf`.
5. Starts the agent and exposes the Workload API on `/tmp/spire-agent/public/api.sock`.

Keep `task run` running in a dedicated terminal.

## Build a2acli

From the `a2acli-skill` directory:

```bash
cd a2acli-skill
task build
```

This creates `a2acli-skill/bin/a2acli`.

## Register a2acli As A SPIRE Workload

From the `multicluster-demo` directory, after the SPIRE agent is already running:

```bash
task register
```

On macOS, the `unix` workload attestor emits `uid`, `user`, `gid`, and `group` selectors. The registration task uses those selectors so the local `a2acli` process can obtain an SVID from the Workload API.

To inspect the entries registered for the current host:

```bash
task show-entries
```

To verify the local Workload API is issuing an SVID:

```bash
task fetch-svid
```

## Configure a2acli

Create a local runtime config at `a2acli-skill/.a2acli.yaml`.

That file is intentionally ignored by Git so it can stay machine-local.

Example:

```yaml
slim:
  endpoint: "${MULTICLUSTER_DEMO_SLIM_ENDPOINT}"
  local-name: "agntcy/cli/a2acli"
  spire:
    socket-path: "/tmp/spire-agent/public/api.sock"
    jwt-audiences:
      - "slim"
```

## Send A Test Request Through The Target Dataplane

From `a2acli-skill`:

```bash
./bin/a2acli list
./bin/a2acli send-message --agent sha256:df721aea --return-immediately "What can you do?"
```

The expected outcome is:

1. `a2acli` initializes the SPIRE provider from the local Workload API socket.
2. `a2acli` connects to the SLIM dataplane configured in `MULTICLUSTER_DEMO_SLIM_ENDPOINT`.
3. The request is accepted and returns a task ID.

## Useful Tasks

```bash
task build
task run
task register
task show-entries
task fetch-svid
task stop
task clean
```

## Shareable Git State

Tracked files in this directory do not contain laptop-specific absolute paths.

Local-only artifacts are kept out of Git:

- `a2acli-skill/.a2acli.yaml`
- `spire/data/`
- `bin/`

The generated file `spire/data/agent.conf` does contain machine-local absolute paths by design, but it is runtime output only and is excluded from Git.

This keeps the workflow PR-ready while still allowing each developer to generate their own local runtime config and local SPIRE state.

## Troubleshooting

If `task run` fails with `Unauthorized` or `ExpiredToken`, refresh AWS credentials and try again.

If the agent fails with `no PEM blocks`, the bootstrap bundle source is wrong. The current Taskfile fetches the PEM bundle directly from the live SPIRE server to avoid that problem.

If `a2acli` fails with `No identity issued`, the workload entry selectors do not match the selectors emitted by the local SPIRE agent. Re-run `task register` after the agent is already running.
