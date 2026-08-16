## Listing and Deleting Routes via the SLIM Control Plane

To accomplish this task you need two `slimctl controller route` commands targeting the node `edge-node-1`.

### Step 1: List all routes for node 'edge-node-1'

Use the `controller route list` command with the `-n` flag to specify the node:

```bash
slimctl controller route list -n edge-node-1
```

This queries the SLIM Control Plane (not the node directly) and returns all routes it knows about for the node identified as `edge-node-1`.

### Step 2: Delete the route for acme/prod/chatbot/3 via http://gateway:9090

Once you have confirmed the route exists in the listing, delete it with:

```bash
slimctl controller route del -n edge-node-1 acme/prod/chatbot/3 via http://gateway:9090
```

### Explanation

- `controller route` commands talk to the SLIM Control Plane, which manages routes across multiple nodes.
- The `-n edge-node-1` (or `--node-id edge-node-1`) flag tells the control plane which node's routing table to operate on.
- The route `acme/prod/chatbot/3` follows the standard `org/namespace/agentname/agentid` format.
- The `via http://gateway:9090` portion identifies the specific next-hop connection to remove for that route.

### Notes

- If your control plane is not running on the default address (`127.0.0.1:46357`), use the `--server` global flag to point at it:

```bash
slimctl --server <control-plane-host:port> controller route list -n edge-node-1
slimctl --server <control-plane-host:port> controller route del -n edge-node-1 acme/prod/chatbot/3 via http://gateway:9090
```

- You can verify the deletion was successful by re-running the list command afterward.
