# slimctl config set

Set a configuration value and write it to the configuration file.

## Usage

```
slimctl config set <SUBCOMMAND> <VALUE>
```

## Subcommands

| Subcommand | Arguments | Description |
|------------|-----------|-------------|
| `server` | `<value>` | Set the default gRPC server address (`host:port`) |
| `timeout` | `<value>` | Set the default request timeout (e.g. `15s`, `1m`) |
| `basic-auth-creds` | `<value>` | Set default Basic auth credentials (`username:password`) |
| `tls-ca-file` | `<value>` | Set the default TLS CA certificate file path |
| `tls-cert` | `<cert_file>` `<key_file>` | Set the default TLS client certificate and key file paths |
| `tls-insecure-skip-verify` | `<value>` | Set TLS insecure skip-verify mode (`true` or `false`) |

## Examples

Set the default server:

```bash
slimctl config set server 192.168.1.10:50051
```

Set the default timeout:

```bash
slimctl config set timeout 30s
```

Configure TLS:

```bash
slimctl config set tls-ca-file /etc/slim/ca.pem
slimctl config set tls-cert /etc/slim/client.pem /etc/slim/client-key.pem
```

Disable TLS certificate verification (development only):

```bash
slimctl config set tls-insecure-skip-verify true
```

## Options

No flags. Arguments are positional (see Subcommands table above).

## Inherited Options

Options inherited from [`slimctl config`](./index.md) and [`slimctl`](../index.md):

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config` | — | `~/.slimctl/config.yaml` | Path to `slimctl` configuration file to write |
| `--server` | `-s` | `127.0.0.1:46358` | gRPC API endpoint |
| `--timeout` | — | `15s` | gRPC request timeout |
| `--basic-auth-creds` | `-b` | — | Basic auth credentials (`username:password`) |
| `--tls.ca_file` | — | — | Path to TLS CA certificate |
| `--tls.cert_file` | — | — | Path to client TLS certificate |
| `--tls.key_file` | — | — | Path to client TLS private key |
| `--tls.insecure_skip_verify` | — | `false` | Skip TLS certificate verification |

## Configuration File Location

`slimctl` searches for its configuration file in this order:

1. Path given by `--config` (error if the file does not exist)
2. `$HOME/.slimctl/config.yaml`
3. `./config.yaml` (current working directory)

Example `~/.slimctl/config.yaml`:

```yaml
server: "127.0.0.1:46358"
timeout: "15s"
tls:
  insecure_skip_verify: false
  ca_file: "/path/to/ca.pem"
  cert_file: "/path/to/client.pem"
  key_file: "/path/to/client-key.pem"
```
