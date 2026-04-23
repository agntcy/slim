# SLIM SPIRE Helm Chart

## Deploying as a downstream SPIRE server

The upstream SPIRE agent uses join token attestation to bootstrap against the root SPIRE server.

### Step 1 — Create a downstream entry for the SPIRE server

This authorises the downstream SPIRE server to obtain an intermediate CA from the upstream agent socket.
Run this on the root SPIRE server: in this example we will install the downstream spire server in the same
trust domain but using a different tenant and cluster name, but you can adjust the SPIFFE ID and selectors as needed.

```bash
spire-server entry create \
  -parentID spiffe://<trust-domain>/tenants/<tenant>/clusters/<cluster-name> \
  -spiffeID spiffe://<trust-domain>/tenants/<tenant>/clusters/<cluster-name>/spire-server \
  -downstream \
  -selector k8s:ns:<namespace> # The namespace of the downstream spire server \
  -selector k8s:sa:<release-name>-server # The service account of the downstream spire server
```

### Step 2 — Mint the join token on the root SPIRE server

Always use `-spiffeID` when generating the token. Without it the agent gets a token-UUID-based SPIFFE ID that changes with every new token, making registration entries unmaintainable.

```bash
spire-server token generate \
  -spiffeID spiffe://<trust-domain>/tenants/<tenant>/clusters/<cluster-name> \
  -ttl 3600
```

### Step 3 — Install the chart

```bash
helm upgrade -i <release-name> . \
  -n <namespace> --create-namespace \
  --set global.spire.clusterName=<cluster-name> \
  --set global.spire.trustDomain=<trust-domain> \
  --set global.spire.upstreamSpireAddress=<upstream-spire-address> \
  --set upstream-spire-agent.joinToken.enabled=true \
  --set upstream-spire-agent.joinToken.value="<token>"
```

The token can also be stored in a pre-existing Secret and referenced via `upstream-spire-agent.joinToken.existingSecret.name`.
