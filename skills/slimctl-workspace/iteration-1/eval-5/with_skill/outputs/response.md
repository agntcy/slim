# Viewing Active Routes and Connections on Your Local SLIM Node

To show all routes and connections currently active on your local SLIM node, run these two commands:

## List all routes

```bash
slimctl node route list
```

## List all connections

```bash
slimctl node connection list
```

By default, these commands connect to `127.0.0.1:46357` (the default local SLIM node control API endpoint). If your node is running on a different address or port, specify it with the `--server` flag:

```bash
slimctl --server <host:port> node route list
slimctl --server <host:port> node connection list
```

## Notes

- The `node` subcommand (aliases: `n`, `instance`, `i`) talks directly to a single SLIM node's local control API.
- Routes are displayed in the format `org/namespace/agentname/agentid` (e.g., `acme/production/assistant/1`).
- The `connection list` command (alias: `conn`) shows all active connections the node has to other SLIM nodes.
- If your node requires authentication, add `--basic-auth-creds user:pass` or the appropriate TLS flags (`--tls.ca_file`, `--tls.cert_file`, `--tls.key_file`).
