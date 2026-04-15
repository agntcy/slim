# Multicluster demo

This directory contains a macOS workflow for:

1. Building and running a local SPIRE agent against the target SPIRE server.
2. Registering the local `a2acli` binary as a SPIRE-authenticated workload.
3. Using `a2acli` to connect to the target SLIM dataplane.

## What Is In This Directory

- `Taskfile.yml`: builds the local SPIRE agent, fetches the bootstrap bundle from the target cluster, generates a join token, starts the agent, registers the `a2acli` workload, and renders a local `a2acli` config.
- `.env.local.example`: tracked example of the local environment file.
- `spire/agent.conf.tmpl`: SPIRE agent config template used to render a local runtime config.
- `a2acli-skill/.a2acli.yaml.tmpl`: template used to render the ignored local `a2acli` config.
- `a2acli-skill/`: the `a2acli` source, local agent cards, and CLI config examples.

## Prerequisites

1. macOS.
2. `go`, `task`, `kubectl`, and `git` installed.
3. Valid credentials with access to the target Kubernetes cluster.

Create a local environment file first:

```bash
cp .env.local.example .env.local
```

Then edit `.env.local` with your local values.

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
2. Regenerates `a2acli-skill/.a2acli.yaml` from `.env.local`.
3. Fetches the target SPIRE bootstrap bundle in PEM format.
4. Generates a join token for `spiffe://<trust-domain>/macos/agent/<hostname>`.
5. Renders a local runtime config under `spire/data/agent.conf`.
6. Starts the agent and exposes the Workload API on `/tmp/spire-agent/public/api.sock`.

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

Generate the ignored local runtime config from `.env.local`:

```bash
task a2acli-config
```

This writes `a2acli-skill/.a2acli.yaml` locally and keeps the endpoint out of tracked files.
`task run` also refreshes this file automatically.

## Send A Test Request Through The Target Dataplane

From `a2acli-skill`:

```bash
task -d .. a2acli-config
./bin/a2acli list
./bin/a2acli send-message --agent sha256:df721aea --return-immediately "What can you do?"
```

The expected outcome is:

1. `a2acli` initializes the SPIRE provider from the local Workload API socket.
2. `a2acli` connects to the endpoint configured in `.env.local`.
3. The request is accepted and returns a task ID.

## Useful Tasks

```bash
task build
task run
task register
task a2acli-config
task show-entries
task fetch-svid
task stop
task clean
```

## Shareable Git State

Tracked files in this directory do not contain laptop-specific absolute paths.

Local-only artifacts are kept out of Git:

- `.env.local`
- `a2acli-skill/.a2acli.yaml`
- `spire/data/`
- `bin/`

The generated file `spire/data/agent.conf` does contain machine-local absolute paths by design, but it is runtime output only and is excluded from Git.

This keeps the workflow PR-ready while still allowing each developer to generate their own local runtime config and local SPIRE state.

## Troubleshooting

If `task run` fails with `Unauthorized` or `ExpiredToken`, refresh AWS credentials and try again.

If the agent fails with `no PEM blocks`, the bootstrap bundle source is wrong. The current Taskfile fetches the PEM bundle directly from the live SPIRE server to avoid that problem.

If `a2acli` fails with `No identity issued`, the workload entry selectors do not match the selectors emitted by the local SPIRE agent. Re-run `task register` after the agent is already running.
