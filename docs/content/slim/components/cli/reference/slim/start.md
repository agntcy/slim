# slimctl slim start

Start a local SLIM node process. The node runs in the foreground until interrupted.

## Usage

```
slimctl slim start [OPTIONS]
```

## Examples

Start using a configuration file:

```bash
slimctl slim start --config data-plane/config/base/server-config.yaml
```

Start with TLS enabled:

```bash
slimctl slim start --config data-plane/config/tls/server-config.yaml
```

Start on a custom endpoint without a config file:

```bash
slimctl slim start --endpoint 127.0.0.1:12345
```

!!! note
    When using `--endpoint`, the node starts without TLS using a minimal generated configuration.

Control log verbosity with the `RUST_LOG` environment variable:

```bash
RUST_LOG=debug slimctl slim start --config data-plane/config/base/server-config.yaml
```

## Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | `-c` | — | Path to a SLIM server YAML configuration file |
| `--endpoint` | — | `127.0.0.1:46357` | Server listen address. Cannot be used together with `--config`. |

`--config` and `--endpoint` are mutually exclusive. If neither is provided, the node starts on `127.0.0.1:46357` with a minimal default configuration.

### Configuration File Examples

Sample configuration files are available in the [`data-plane/config/`](https://github.com/agntcy/slim/tree/main/data-plane/config) directory of the repository:

| Directory | Description |
|-----------|-------------|
| [`base/`](https://github.com/agntcy/slim/tree/main/data-plane/config/base) | Plaintext, no authentication |
| [`tls/`](https://github.com/agntcy/slim/tree/main/data-plane/config/tls) | TLS-secured server |
| [`mtls/`](https://github.com/agntcy/slim/tree/main/data-plane/config/mtls) | Mutual TLS authentication |
| [`basic-auth/`](https://github.com/agntcy/slim/tree/main/data-plane/config/basic-auth) | HTTP Basic authentication |
| [`jwt-auth-rsa/`](https://github.com/agntcy/slim/tree/main/data-plane/config/jwt-auth-rsa) | JWT with RSA signing |
| [`spire/`](https://github.com/agntcy/slim/tree/main/data-plane/config/spire) | SPIFFE/SPIRE workload identity |
| [`telemetry/`](https://github.com/agntcy/slim/tree/main/data-plane/config/telemetry) | OpenTelemetry integration |

## Inherited Options

Options inherited from [`slimctl slim`](./index.md):

No additional options.

Options inherited from [`slimctl`](../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | — | `~/.slimctl/config.yaml` | Path to `slimctl` **client** configuration file |
| `--server` | `-s` | `127.0.0.1:46358` | gRPC API endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

!!! note
    The `--config` / `-c` option under `slimctl slim start` is the **server** configuration file for the SLIM node. The `--config` option inherited from `slimctl` is the **client** configuration file for `slimctl` itself. These are distinct.
