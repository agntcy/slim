# Deployment

## AWS

Get access to the cluster `slim-root` in the account
`outshift-common-dev`.

Then run the following commands to deploy the root SPIRE server:

```bash
cd root
helm upgrade --install spire-crds spiffe/spire-crds \
  -n spire-system --create-namespace
helm upgrade --install spire . \
  -n spire-system --create-namespace -f values.yaml
```

Once deployed, you should see the following pods up and running:

```bash
kubectl get pods -n spire-system
NAME                                              READY   STATUS
spiffe-csi-driver-downstream-fc9jh                2/2     Running
spiffe-csi-driver-downstream-snrrp                2/2     Running
spiffe-csi-driver-downstream-tbpgp                2/2     Running
spiffe-csi-driver-upstream-9l228                  2/2     Running
spiffe-csi-driver-upstream-p8vxc                  2/2     Running
spiffe-csi-driver-upstream-r7795                  2/2     Running
spiffe-oidc-discovery-provider-8659c984bb-npk4x   2/2     Running
spire-agent-downstream-8x5k4                      1/1     Running
spire-agent-downstream-dhsrm                      1/1     Running
spire-agent-downstream-fddgf                      1/1     Running
spire-agent-upstream-6lvwb                        1/1     Running
spire-agent-upstream-6qwp2                        1/1     Running
spire-agent-upstream-t8h5d                        1/1     Running
spire-external-server-0                           2/2     Running
spire-internal-server-0                           2/2     Running
spire-root-server-0                               2/2     Running
```

## On-Prem

The on-prem deployment is managed by ArgoCD. Values are
included here for reference.

### 1. Configure the upstream trust bundle

The on-prem deployment needs the trust bundle from the
upstream (root) SPIRE server. Retrieve it by running the
following command against the **root cluster**:

```bash
kubectl -n spire-system get configmap \
  spire-bundle-upstream \
  -o jsonpath='{.data.bundle\.spiffe}'
```

Set the output as the `upstreamBundle` value in the Helm
chart.

### 2. Register the on-prem node in the root SPIRE server

Switch your kubectl context to the **root cluster** and
create a registration entry for the on-prem node:

```bash
kubectl exec -n spire-system \
  spire-external-server-0 \
  -c external-spire-server -- \
  ./bin/spire-server entry create \
    -parentID spiffe://slim.dev.outshift.ai/nested/upstream/gls-cluster-0/tf-kubeflow-node-0 \
    -spiffeID spiffe://slim.dev.outshift.ai/ns/spire-system/sa/spire-internal-server \
    -selector k8s:ns:spire-system \
    -downstream
```

> **Note:** We run a single upstream agent in the on-prem
> cluster, pinned to node 0. If you need multiple agents,
> create an entry for each one with the appropriate
> `parentID` and `spiffeID`.

Verify the entry was created:

```bash
kubectl exec -n spire-system \
  spire-external-server-0 \
  -c external-spire-server -- \
  ./bin/spire-server entry show \
    -parentID spiffe://slim.dev.outshift.ai/nested/upstream/gls-cluster-0/tf-kubeflow-node-0 \
    -spiffeID spiffe://slim.dev.outshift.ai/ns/spire-system/sa/spire-internal-server
```

### 3. Generate a join token

Still on the **root cluster**, generate a join token for the
on-prem agent:

```bash
kubectl exec -n spire-system \
  spire-external-server-0 \
  -c external-spire-server -- \
  ./bin/spire-server token generate \
    -spiffeID spiffe://slim.dev.outshift.ai/nested/upstream/gls-cluster-0/tf-kubeflow-node-0 \
    -ttl 3600
```

Save the generated token — you will need it in the next step.

### 4. Deploy the on-prem SPIRE chart

This step is handled by ArgoCD, but you can also deploy
manually. Switch your kubectl context to the **on-prem
cluster** and run:

```bash
cd nested
helm upgrade --install spire-crds spiffe/spire-crds \
  -n spire-system --create-namespace
helm upgrade --install spire . \
  -n spire-system --create-namespace -f values.yaml
```

### 5. Inject the join token

After deployment, the upstream agent will not start because
the Helm chart does not support setting the join token
directly. Patch the ConfigMap manually:

```bash
# Set the token from step 3
JOIN_TOKEN=XXXXX

# Extract the current agent config
CURRENT=$(kubectl -n spire-system get configmap \
  spire-agent-upstream \
  -o jsonpath='{.data.agent\.conf}')

# Add the join_token field to the agent block
UPDATED=$(echo "$CURRENT" | \
  jq --arg token "$JOIN_TOKEN" \
  '.agent.join_token = $token')

# Patch the ConfigMap
kubectl -n spire-system patch configmap \
  spire-agent-upstream --type merge \
  -p "{\"data\":{\"agent.conf\":$(echo "$UPDATED" | jq -Rs .)}}"
```

After a moment, `spire-agent-upstream` should become healthy.
Once it is running, `spire-internal-server` will also come up
because it depends on the upstream agent for node attestation.

### Troubleshooting

- **ArgoCD race condition:** If you deployed with ArgoCD, it
  may overwrite the patched ConfigMap and restore the original
  (without the join token). You may need to temporarily
  disable auto-sync or re-apply the patch.

- **Pods stuck in a non-ready state:** Delete the affected
  pods so they are recreated with the updated configuration:

  ```bash
  kubectl -n spire-system delete pod spire-agent-upstream-XXXXX
  kubectl -n spire-system delete pod spire-internal-server-0
  ```

  You can do the same with the `spire-agent-downstream` pods
  to speed up convergence.
