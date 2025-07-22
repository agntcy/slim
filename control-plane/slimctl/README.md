# slimctl

`slimctl` is the command-line interface for the SLIM control plane.

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

`server` endpoint can the Controller endpoint of a SLIM instance, in case of connecting directly to a SLIM instance or an endpoint of Control Plane endpoint in case of using `slimctl cp` sub-commands.

## Commands 

### Connected directly to a SLIM instance

* `slimctl connection list` List connection on a SLIM instance.
* `slimctl route list` List routes on a SLIM instance.
* `slimctl route add <organization/namespace/agentName/agentId> via <config_file>` Add a route to the SLIM instance.
* `slimctl route del <organization/namespace/agentName/agentId> via <host:port>` Delete a route from the SLIM instance.


### Connected through Control Plane

* `slimctl cp connection list --node-id=<slim_node_id>` List connection on a SLIM instance.
* `slimctl cp route list --node-id=<slim_node_id>` List routes on a SLIM instance.
* `slimctl cp route add <organization/namespace/agentName/agentId> via <config_file> --node-id=<slim_node_id>` Add a route to the SLIM instance.
* `slimctl cp route del <organization/namespace/agentName/agentId> via <host:port> --node-id=<slim_node_id>` Delete a route from the SLIM instance.


* `slimctl version` Print version information.

Run `slimctl <command> --help` for more details on flags and usage.

## Examples

```bash
# Add a new route
slimctl route add org/default/alice/0 via localhost:46367

# Delete an existing route
slimctl route del org/default/alice/0 via localhost:46367
```