# SLIM Controller Reference

Command-line reference for `slimctl`, the unified CLI for managing SLIM nodes, the
control plane, channel manager, and local development instances.

## Global options

These flags apply to all subcommands (may appear before or after the subcommand):

| Flag | Environment variable | Description |
|------|---------------------|-------------|
| `--config <file>` | `SLIMCTL_CONFIG` | Path to slimctl client configuration file |
| `--server <host:port>` | `SLIMCTL_COMMON_OPTS_SERVER` | Override the gRPC endpoint |
| `--timeout <duration>` | `SLIMCTL_COMMON_OPTS_TIMEOUT` | Request timeout (e.g. `15s`, `1m`) |
| `-b`, `--basic-auth-creds <user:pass>` | `SLIMCTL_COMMON_OPTS_BASIC_AUTH_CREDS` | HTTP basic auth credentials |
| `--tls.insecure_skip_verify` | `SLIMCTL_COMMON_OPTS_TLS_INSECURE_SKIP_VERIFY` | Skip TLS certificate verification |
| `--tls.ca_file <path>` | `SLIMCTL_COMMON_OPTS_TLS_CA_FILE` | CA certificate file |
| `--tls.cert_file <path>` | `SLIMCTL_COMMON_OPTS_TLS_CERT_FILE` | Client certificate file |
| `--tls.key_file <path>` | `SLIMCTL_COMMON_OPTS_TLS_KEY_FILE` | Client key file |

### Default endpoints

When `--server` is not set and the config file has no endpoint, slimctl uses a
per-subcommand default:

| Subcommand | Default endpoint |
|------------|------------------|
| `slimctl controller` | `127.0.0.1:50051` (control plane northbound API) |
| `slimctl node` | `127.0.0.1:46358` (node control API) |
| `slimctl channel-manager` | `127.0.0.1:10356` (channel manager API) |
| `slimctl slim start` (no config) | `127.0.0.1:46357` (data plane) |

### Configuration file

slimctl looks for configuration at:

1. `--config` / `SLIMCTL_CONFIG` if set
2. `$HOME/.slimctl/config.yaml`
3. `./config.yaml`

Example `~/.slimctl/config.yaml`:

```yaml
endpoint: "127.0.0.1:50051"
request_timeout: 15s
connect_timeout: 15s
tls:
  insecure: true
```

Use `slimctl config list` to print the resolved configuration and
`slimctl config set` to update values:

```bash
slimctl config set server 127.0.0.1:50051
slimctl config set timeout 30s
slimctl config set basic-auth-creds admin:secret
slimctl config set tls-ca-file /path/to/ca.pem
slimctl config set tls-cert /path/to/cert.pem /path/to/key.pem
slimctl config set tls-insecure-skip-verify true
```

## `slim` — Local SLIM instances

Run standalone SLIM instances for development and testing.

**Start with a configuration file:**

```bash
slimctl slim start --config config/base/server-config.yaml
slimctl slim start --config config/tls/server-config.yaml
```

**Quick start with a custom listening endpoint:**

```bash
slimctl slim start --endpoint 127.0.0.1:12345
```

!!! note
    `--config` and `--endpoint` are mutually exclusive. The server starts without
    TLS when using `--endpoint`.

**Control log level:**

```bash
RUST_LOG=debug slimctl slim start --config config/base/server-config.yaml
```

The log level can also be set in the configuration file via `tracing.log_level`.
See [config/base/server-config.yaml](https://github.com/agntcy/slim/blob/main/config/base/server-config.yaml).

| Flag | Environment variable | Description |
|------|---------------------|-------------|
| `-c`, `--config <file>` | — | Path to SLIM node YAML configuration |
| `--endpoint <addr>` | `SLIMCTL_SLIM_ENDPOINT` | Listen address (no config file) |

**Example configs** in [config/](https://github.com/agntcy/slim/tree/main/config):

- [base](https://github.com/agntcy/slim/tree/main/config/base) — basic insecure configuration
- [tls](https://github.com/agntcy/slim/tree/main/config/tls) — TLS-enabled server
- [mtls](https://github.com/agntcy/slim/tree/main/config/mtls) — mutual TLS
- [basic-auth](https://github.com/agntcy/slim/tree/main/config/basic-auth) — HTTP Basic authentication
- `jwt-auth-*` — JWT ([RSA](https://github.com/agntcy/slim/tree/main/config/jwt-auth-rsa),
  [ECDSA](https://github.com/agntcy/slim/tree/main/config/jwt-auth-ecdsa),
  [HMAC](https://github.com/agntcy/slim/tree/main/config/jwt-auth-hmac))
- [spire](https://github.com/agntcy/slim/tree/main/config/spire) — SPIFFE/SPIRE workload identity
- [proxy](https://github.com/agntcy/slim/tree/main/config/proxy) — HTTP proxy
- [telemetry](https://github.com/agntcy/slim/tree/main/config/telemetry) — OpenTelemetry

## `controller` — Control plane management

Talks to the control plane northbound API (default `127.0.0.1:50051`). Routes are
**reconciled** from topology configuration (links and segments) — they are not added
or deleted imperatively via the CLI. See [Control Plane Configuration](./slim-control-plane-config.md).

### `controller node`

List nodes registered with the control plane:

```bash
slimctl controller node list
```

### `controller connection`

List active connections on a node:

```bash
slimctl controller connection list -n slim/0
```

`-n` / `--node-id` is required.

### `controller route`

List routes. Without `-n`, lists all routes at the controller; with `-n`, shows
routes on a specific node:

```bash
# All applied routes at the controller
slimctl controller route list

# Routes on a specific node
slimctl controller route list -n slim/0

# Filter by origin or target node
slimctl controller route list -o slim/a -t slim/b

# Include pending, failed, and deleted routes
slimctl controller route list -a
```

| Flag | Description |
|------|-------------|
| `-n`, `--node-id` | Per-node view (conflicts with `-o`/`-t`/`-a`) |
| `-o`, `--origin-node-id` | Filter by source node (controller-wide view) |
| `-t`, `--target-node-id` | Filter by destination node (controller-wide view) |
| `-a`, `--all` | Show all statuses (default: applied only) |

### `controller link`

Manage topology links between groups (API-managed mode only):

```bash
slimctl controller link list
slimctl controller link list -o group-a -t group-b
slimctl controller link list -a

slimctl controller link add group-a group-b
slimctl controller link add group-a group-b -s my-segment

slimctl controller link remove group-a group-b
slimctl controller link remove group-a group-b -s my-segment
```

| Flag | Description |
|------|-------------|
| `-s`, `--segment` | Segment name (default: `default`) |
| `-o`, `-t`, `-a` | Same as `route list` |

### `controller segment`

Manage routing segments (API-managed mode only):

```bash
slimctl controller segment list
slimctl controller segment add my-segment
slimctl controller segment remove my-segment
```

### `controller group`

List groups and their member nodes:

```bash
slimctl controller group list
```

## `node` — Direct node management

Connect directly to a SLIM node's control endpoint, bypassing the central control
plane. Uses the node default endpoint (`127.0.0.1:46358`) unless overridden with
`--server`.

```bash
slimctl --server 127.0.0.1:46358 node route list
slimctl --server 127.0.0.1:46358 node connection list
```

## `channel-manager` — Group channel management {#channel-manager-group-channel-management}

Manage SLIM group channels and participants via the channel manager service
(default `127.0.0.1:10356`). Requires a running
[channel manager](./slim-channel-manager.md) instance.

```bash
slimctl channel-manager create-channel org/ns/my-channel
slimctl channel-manager create-channel org/ns/my-channel --disable-mls

slimctl channel-manager add-participant org/ns/my-channel org/ns/client-1
slimctl channel-manager delete-participant org/ns/my-channel org/ns/client-1

slimctl channel-manager list-channels
slimctl channel-manager list-participants org/ns/my-channel

slimctl channel-manager delete-channel org/ns/my-channel
```

Aliases: `cc`, `ap`, `dp`, `dc`, `lc`, `lp`. The subcommand itself can be
abbreviated as `cm`.

## `bench` — Performance benchmarks

Benchmark messaging performance against a running SLIM server.

```bash
# Terminal 1 — start subscribers
slimctl bench sub --count 4 --msgs 100000 --size 512

# Terminal 2 — run publishers
slimctl bench pub --count 4 --msgs 100000 --size 512

# Request-reply latency
slimctl bench sub --reply
slimctl bench pub --request -n 10000

# Group (channel) benchmark
slimctl bench channel sub --count 2
slimctl bench channel pub --count 1 --reliable
```

Common flags for `sub` and `pub`:

| Flag | Default | Description |
|------|---------|-------------|
| `--count` | `1` | Number of concurrent apps |
| `-n`, `--msgs` | `100000` | Total messages |
| `--size` | `128` | Payload size (supports `kb`, `mb`, `kib`, etc.) |
| `--server` | `http://localhost:46357` | SLIM server URL |
| `--secret` | (built-in test secret) | Shared secret (min 32 chars) |
| `--prefix` | `bench/test` | Name prefix (`org/namespace` format) |
| `--csv` | — | Optional CSV output path |
| `--start-index` | `0` | Starting index for app names |

`bench sub` only: `--reply` (echo mode). `bench pub` only: `--request`, `--reliable`.

## `version`

Display version and build information:

```bash
slimctl version
```

## Getting help

```bash
slimctl --help
slimctl controller --help
slimctl controller route --help
slimctl channel-manager --help
slimctl bench --help
```

## Usage example: inspect topology and routes

```bash
# List registered nodes
slimctl controller node list

# List groups and their members
slimctl controller group list

# List topology links
slimctl controller link list

# List reconciled routes for a node
slimctl controller route list -n slim/a
```

For client connection configuration used in SLIM node YAML, see the
[client-config schema](https://github.com/agntcy/slim/blob/main/crates/config/src/schema/client-config.schema.json).
