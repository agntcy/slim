# slimctl

`slimctl` is the command-line interface for the SLIM controller.

## Configuration

`slimctl` supports configuration via a file, environment variables, and command-line flags.

### Config file

By default, `slimctl` looks for a config file at `$HOME/.slimctl/config.yaml` or in the current working directory. Example `config.yaml`:

```yaml
server: "127.0.0.1:46358"
timeout: "10s"
tls:
  insecure: false
  ca_file: "/path/to/ca.pem"
  cert_file: "/path/to/client.pem"
  key_file: "/path/to/client.key"
```

`server` endpoint should point to a [SLIM Control](https://github.com/agntcy/slim/tree/main/control-plane/control-plane) endpoint which is a central service managing SLIM node configurations.
SLIM nodes can be configured to expose a Controller endpoint of a SLIM instance, `slimctl` can connect to this endpoint to manage the SLIM instance directly by using `slimctl node-control` sub-command.
In this case `server` should point to the SLIM instance endpoint.

## Commands 

* `slimctl connection list --node-id=<slim_node_id>` List connection on a SLIM instance.
* `slimctl route list --node-id=<slim_node_id>` List routes on a SLIM instance.
* `slimctl route add <organization/namespace/agentName/agentId> via <config_file> --node-id=<slim_node_id>` Add a route to the SLIM instance.
* `slimctl route del <organization/namespace/agentName/agentId> via <host:port> --node-id=<slim_node_id>` Delete a route from the SLIM instance.

* `slimctl version` Print version information.

Run `slimctl <command> --help` for more details on flags and usage.

## Examples

### Create, delete route

```bash
# Add a new route
cat >> connection_config.json <<EOF
{
"endpoint": "http://127.0.0.1:46357"
}
slimctl route add org/default/alice/0 via connection_config.json


# Delete an existing route
slimctl route del org/default/alice/0 via http://localhost:46367
```

> For full reference of connection_config.json checkout [client-config-schema.json](https://github.com/agntcy/slim/blob/main/data-plane/core/config/src/grpc/schema/client-config.schema.json)

### Manager routes & list connections connecting directly to a SLIM node

* `slimctl node-connect connection list --server=<node_control_endpoint>` List connection on a SLIM instance.
* `slimctl node-connect route list --server=<node_control_endpoint>` List routes on a SLIM instance.
* `slimctl node-connect route add <organization/namespace/agentName/agentId> via <config_file> --server=<node_control_endpoint>` Add a route to the SLIM instance.
* `slimctl node-connect route del <organization/namespace/agentName/agentId> via <host:port> --server=<node_control_endpoint>` Delete a route from the SLIM instance.
