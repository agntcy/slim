# SLIM CLI

`slimctl` is the command-line tool for operating SLIM. It lets you start and manage SLIM nodes, inspect connections and routes, manage Controller configuration, and interact with the Channel Manager — all from a single binary.

## What You Can Do With slimctl

- **Start a SLIM node** — launch a local SLIM Data Plane instance for development and testing using any configuration file
- **Manage routes** — add, list, and remove message routes between nodes via the Controller
- **Inspect connections** — list active connections, check node status, and monitor traffic
- **Manage channels** — create channels, invite participants, and manage group membership via the Channel Manager
- **Run benchmarks** — publish and subscribe throughput tests for performance validation

## In This Section

- [Installation](./install.md) — Download pre-built binaries, install via Homebrew, or build from source
- [Command Reference](./reference/index.md) — Full reference for all `slimctl` commands and flags

## Related

- [SLIM Controller](../controller/index.md) — The control plane component `slimctl` manages
- [SLIM Channel Manager](../channel-manager/index.md) — Manage group channels with `slimctl channel-manager`
