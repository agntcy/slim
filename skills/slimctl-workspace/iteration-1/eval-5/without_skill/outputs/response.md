# Viewing Routes and Connections on Your Local SLIM Node

To show all routes and connections currently active on your local SLIM node, you need to use the `slimctl` CLI tool with the `node` subcommand. There are two separate commands -- one for routes and one for connections.

## List All Active Routes

```bash
slimctl node route list
```

This connects to your local SLIM node's gRPC control API and retrieves the current subscription/routing table. The output shows each route entry in the format:

```
<org>/<namespace>/<agent_type> id=<agent_id> local=[...] remote=[...]
```

Each entry includes:
- The hierarchical name components (organization, namespace, agent type)
- The agent ID
- Local connections associated with that route (with connection ID, config data, link ID, and direction)
- Remote connections associated with that route

You can also use the alias `ls` instead of `list`:

```bash
slimctl node route ls
```

## List All Active Connections

```bash
slimctl node connection list
```

This retrieves all active connections on the node. The output shows each connection in the format:

```
id=<connection_id> direction=<Inbound|Outbound> link_id=<link_id> <config_data>
```

You can also use the alias `conn` for `connection`:

```bash
slimctl node conn list
```

## Configuration

By default, `slimctl` connects to `localhost:46357` (the default SLIM gRPC control API endpoint). If your node is running on a different address, you can specify it with:

```bash
slimctl --server <host:port> node route list
slimctl --server <host:port> node connection list
```

Or set it persistently:

```bash
slimctl config set server <host:port>
```

If your node is running without TLS (common in local development), add the `--tls.insecure` flag:

```bash
slimctl --tls.insecure node route list
slimctl --tls.insecure node connection list
```

## Combined View

To see both routes and connections together, simply run both commands:

```bash
slimctl node route list
slimctl node connection list
```

There is no single command that combines both outputs into one view -- they must be queried separately.
