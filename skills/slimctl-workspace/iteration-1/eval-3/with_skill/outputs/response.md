# Configuring slimctl for Persistent TLS Connection

To make slimctl always connect to your SLIM node at `myhost.internal:9090` with mTLS using your specified certificates, run the following `slimctl config set` commands. These persist the settings to `~/.slimctl/config.yaml` so they apply to every future invocation without needing to pass flags each time.

```bash
slimctl config set server myhost.internal:9090
slimctl config set tls-ca-file /etc/pki/ca.pem
slimctl config set tls-cert-file /etc/pki/client.pem
slimctl config set tls-key-file /etc/pki/client.key
```

## What each command does

| Command | Effect |
|---------|--------|
| `slimctl config set server myhost.internal:9090` | Sets the default gRPC endpoint to your SLIM node |
| `slimctl config set tls-ca-file /etc/pki/ca.pem` | Tells slimctl to use your CA certificate for verifying the server |
| `slimctl config set tls-cert-file /etc/pki/client.pem` | Sets the client certificate for mutual TLS authentication |
| `slimctl config set tls-key-file /etc/pki/client.key` | Sets the client private key for mutual TLS authentication |

## Verifying the configuration

After running the commands above, confirm your settings with:

```bash
slimctl config list
```

This will display all persisted values from `~/.slimctl/config.yaml`.

## Testing the connection

Try listing routes to verify connectivity:

```bash
slimctl node route list
```

If the connection succeeds, slimctl is correctly configured to use TLS with your CA and client certificates against `myhost.internal:9090`.

## Notes

- These settings are stored in `~/.slimctl/config.yaml` and apply to all commands by default.
- You can still override any setting for a single invocation using CLI flags (e.g., `--server`, `--tls.ca_file`, `--tls.cert_file`, `--tls.key_file`) or environment variables (e.g., `$SLIMCTL_COMMON_OPTS_SERVER`).
- Since you are providing a CA file, client cert, and client key, TLS is implicitly enabled (no need to set `tls-insecure` to false -- that is only the default for plain HTTP/2 connections).
