# agpctl

`agpctl` is the command-line interface for the AGP control plane.

## Configuration

`agpctl` supports configuration via a file, environment variables, and command-line flags.

### Config file

By default, `agpctl` looks for a config file at `$HOME/.agpctl/config.yaml` or in the current working directory. Example `config.yaml`:

```yaml
server: "127.0.0.1:46358"
timeout: "10s"
tls:
  insecure: false
  ca_file: "/path/to/ca.pem"
  cert_file: "/path/to/client.pem"
  key_file: "/path/to/client.key"
```

## Commands

* `agpctl route list` List routes on a gateway.
* `agpctl route add <organization/namespace/agentName/agentId> via <host:port>` Add a route to the gateway.
* `agpctl route del <organization/namespace/agentName/agentId> via <host:port>` Delete a route from the gateway.
* `agpctl version` Print version information.

Run `agpctl <command> --help` for more details on flags and usage.

## Examples

```bash
# Add a new route
agpctl route add org/default/alice/0 via localhost:46367

# Delete an existing route
agpctl route del org/default/alice/0 via localhost:46367
```