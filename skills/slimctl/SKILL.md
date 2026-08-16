---
name: slimctl
description: >
  Use this skill whenever the user mentions slimctl or SLIM nodes. slimctl is a
  proprietary CLI — not a standard system tool — for managing SLIM (Secure
  Low-Latency Interactive Messaging) infrastructure. Always invoke for: starting
  or configuring a local SLIM node, connecting slimctl to a remote server,
  setting up TLS or mTLS for slimctl, configuring slimctl via flags/env
  vars/config files, adding or listing routes on a SLIM node, querying the SLIM
  control plane (nodes, routes, channels, participants), or troubleshooting
  slimctl connection errors. The primary triggers are the words "slimctl" or
  "SLIM node/control plane" anywhere in the query.
---

# slimctl — SLIM Control CLI

> **Note:** slimctl is a proprietary tool with its own specific command syntax.
> Do not guess or invent commands from general knowledge — use only the
> reference below.

slimctl manages SLIM nodes and the SLIM Control Plane over gRPC. There are two
target audiences for commands:

- **`node`** — talks directly to a single SLIM node's local control API
- **`controller`** — talks to the SLIM Control Plane, which manages multiple nodes

## Defaults

| Setting | Default |
|---|---|
| Server | `127.0.0.1:46357` |
| Timeout | `15s` |
| TLS | **insecure = true** (plain HTTP/2, no TLS) |
| Config file | `~/.slimctl/config.yaml` |

> **Important:** `tls-insecure` is `true` by default. Providing cert/CA files alone does NOT
> enable TLS — you must also explicitly set `tls-insecure false`, otherwise the connection
> stays plain HTTP/2 and the cert files are ignored.

## Global flags (apply to every command)

```
--config / $SLIMCTL_CONFIG              path to config file
-s, --server / $SLIMCTL_COMMON_OPTS_SERVER    gRPC endpoint  host:port
-b, --basic-auth-creds / $SLIMCTL_COMMON_OPTS_BASIC_AUTH_CREDS  user:pass
    --timeout / $SLIMCTL_COMMON_OPTS_TIMEOUT  duration e.g. 15s, 1m
    --tls.insecure                       disable TLS (plain HTTP/2)
    --tls.insecure_skip_verify           skip cert verification
    --tls.ca_file                        path to CA cert
    --tls.cert_file                      path to client cert
    --tls.key_file                       path to client key
```

---

## Route format

Routes are written as four slash-separated components:

```
org/namespace/agentname/agentid
```

Example: `acme/production/assistant/1`

---

## Commands

### version

```bash
slimctl version
```

### config

Persistent settings live in `~/.slimctl/config.yaml`. CLI flags and env vars
always override the file for a single invocation.

```bash
slimctl config list
slimctl config set server      <host:port>
slimctl config set timeout     <duration>        # e.g. 30s, 2m
slimctl config set tls-insecure <true|false>
slimctl config set tls-insecure-skip-verify <true|false>
slimctl config set tls-ca-file  <path>
slimctl config set tls-cert-file <path>
slimctl config set tls-key-file  <path>
slimctl config set basic-auth-creds <user:pass>
```

### node (aliases: `n`, `instance`, `i`)

Talks directly to one SLIM node. Use the global `--server` flag to point at it.

```bash
# Routes
slimctl node route list
slimctl node route add  <org/ns/agent/id>  via  <connection-config.json>
slimctl node route del  <org/ns/agent/id>  via  <http|https://host:port>

# Connections
slimctl node connection list      # alias: conn
```

The connection config JSON for `route add` needs at minimum:
```json
{ "endpoint": "http://host:port" }
```

### controller (aliases: `c`, `ctrl`)

Talks to the SLIM Control Plane.

```bash
# Nodes
slimctl controller node list

# Connections  (require -n / --node-id)
slimctl controller connection list -n <node-id>

# Routes  (require -n / --node-id)
slimctl controller route list    -n <node-id>
slimctl controller route add     -n <node-id>  <org/ns/agent/id>  via  <dest-node>
slimctl controller route del     -n <node-id>  <org/ns/agent/id>  via  <http|https://host:port>
slimctl controller route outline [-o <src-node>] [-t <dst-node>]

# Links
slimctl controller link list

# Channels (MLS groups)
slimctl controller channel create  <moderators=alice,bob>
slimctl controller channel delete  <channel-name>
slimctl controller channel list

# Participants
slimctl controller participant add    <name>  -c <channel-id>
slimctl controller participant delete <name>  -c <channel-id>
slimctl controller participant list          -c <channel-id>
```

### slim (alias: `s`)

Starts a local SLIM node in the foreground. `--config` and `--endpoint` are mutually exclusive.

```bash
slimctl slim start                          # default endpoint 127.0.0.1:46357
slimctl slim start --endpoint 0.0.0.0:9090
slimctl slim start -c /etc/slim/config.yaml
```

---

## Common workflows

### Point slimctl at a different node

```bash
# One-off
slimctl --server myhost:9090 node route list

# Persist
slimctl config set server myhost:9090
```

### mTLS connection

Remember: `tls-insecure` defaults to `true`, so you must turn it off or TLS will not be used
even when cert files are provided.

**Persist mTLS settings** (recommended):
```bash
slimctl config set tls-insecure false          # REQUIRED — disables plain HTTP/2 mode
slimctl config set tls-ca-file  /etc/ssl/ca.pem
slimctl config set tls-cert-file /etc/ssl/client.pem
slimctl config set tls-key_file  /etc/ssl/client.key
slimctl config set server myhost:9090
```

**One-off mTLS flag**:
```bash
slimctl --tls.insecure=false \
        --tls.ca_file /etc/ssl/ca.pem \
        --tls.cert_file /etc/ssl/client.pem \
        --tls.key_file  /etc/ssl/client.key \
        node route list
```

### Add a route directly on a node

```bash
# 1. Create a connection config file
cat > /tmp/conn.json <<'EOF'
{ "endpoint": "http://remote-node:8080" }
EOF

# 2. Add the route
slimctl node route add acme/prod/assistant/1 via /tmp/conn.json
```

### Manage a channel through the control plane

```bash
# Create channel with a moderator
slimctl controller channel create moderators=alice

# Add participants
slimctl controller participant add bob   -c <channel-id>
slimctl controller participant add carol -c <channel-id>

# List participants
slimctl controller participant list -c <channel-id>
```
