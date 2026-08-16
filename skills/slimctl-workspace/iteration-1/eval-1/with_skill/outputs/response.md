## Adding a Route on a SLIM Node with slimctl

To add a route for agent `acme/production/assistant/1` that goes via the remote node at `http://10.0.0.5:8080`, you need two steps:

### Step 1: Create a connection config file

Create a JSON file that specifies the remote node endpoint:

```bash
cat > /tmp/conn.json <<'EOF'
{ "endpoint": "http://10.0.0.5:8080" }
EOF
```

### Step 2: Add the route

Use the `slimctl node route add` command, providing the agent route and the path to the connection config file:

```bash
slimctl node route add acme/production/assistant/1 via /tmp/conn.json
```

### Notes

- The route format is `org/namespace/agentname/agentid`, so `acme/production/assistant/1` breaks down as org=`acme`, namespace=`production`, agent name=`assistant`, agent id=`1`.
- By default, slimctl connects to the local SLIM node at `127.0.0.1:46357`. If your node is running on a different address, use the `--server` flag:

```bash
slimctl --server <your-node-host:port> node route add acme/production/assistant/1 via /tmp/conn.json
```

- You can verify the route was added by listing routes:

```bash
slimctl node route list
```

- To remove the route later:

```bash
slimctl node route del acme/production/assistant/1 via http://10.0.0.5:8080
```
