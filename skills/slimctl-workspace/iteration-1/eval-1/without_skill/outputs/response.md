# Adding a Route with slimctl

To add a route for agent `acme/production/assistant/1` that goes via a remote node at `http://10.0.0.5:8080`, you need to use the `slimctl node route add` command. This command requires a JSON connection config file that specifies the remote endpoint.

## Steps

### 1. Create a connection config file

Create a JSON file that contains the endpoint of the remote node:

```bash
cat > /tmp/conn.json <<'EOF'
{ "endpoint": "http://10.0.0.5:8080" }
EOF
```

### 2. Add the route

```bash
slimctl node route add acme/production/assistant/1 via /tmp/conn.json
```

This tells your local SLIM node to route messages destined for agent `acme/production/assistant/1` through the remote node at `http://10.0.0.5:8080`.

## Syntax breakdown

The `node route add` command syntax is:

```
slimctl node route add <org/namespace/agentname/agentid> via <path-to-config.json>
```

- The route is specified as four slash-separated components: `org/namespace/agentname/agentid`
- The keyword `via` is a literal separator
- The last argument is a path to a JSON file containing at minimum an `"endpoint"` field

## Pointing slimctl at a specific node

By default, slimctl connects to `127.0.0.1:46357`. If your local SLIM node is running on a different address, use the `--server` flag or configure it persistently:

```bash
# One-off override
slimctl --server localhost:46357 node route add acme/production/assistant/1 via /tmp/conn.json

# Or persist the setting
slimctl config set server localhost:46357
```

## TLS considerations

If your node's control API does not use TLS (common in development), slimctl defaults to insecure (plain HTTP/2) mode. If TLS is enabled, you may need flags like `--tls.insecure_skip_verify` or provide CA/cert/key files via `--tls.ca_file`, `--tls.cert_file`, and `--tls.key_file`.

## Verifying the route

After adding the route, you can verify it was applied:

```bash
slimctl node route list
```

## Removing the route later

To remove the route:

```bash
slimctl node route del acme/production/assistant/1 via http://10.0.0.5:8080
```

Note that for deletion, you specify the endpoint URL directly (not a config file).
