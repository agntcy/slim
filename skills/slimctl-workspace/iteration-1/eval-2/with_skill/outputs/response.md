# Managing Routes for 'edge-node-1' via the Control Plane

## 1. List all routes for node 'edge-node-1'

```bash
slimctl controller route list -n edge-node-1
```

This queries the SLIM Control Plane for all routes associated with the node `edge-node-1`.

## 2. Delete the route for acme/prod/chatbot/3 via http://gateway:9090

```bash
slimctl controller route del -n edge-node-1 acme/prod/chatbot/3 via http://gateway:9090
```

This removes the specific route entry for `acme/prod/chatbot/3` that is routed through `http://gateway:9090` on the `edge-node-1` node.

---

**Notes:**

- Both commands use the `controller` subcommand (aliases: `c`, `ctrl`) since you are working with the control plane, not a single node's local API.
- The `-n` / `--node-id` flag specifies which node's routes to manage.
- If the control plane is not running at the default address (`127.0.0.1:46357`), add the `--server` flag:
  ```bash
  slimctl --server <control-plane-host:port> controller route list -n edge-node-1
  ```
