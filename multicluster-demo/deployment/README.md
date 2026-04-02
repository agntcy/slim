# Deployment

## AWS

Get access to the cluster `slim-root` in the account `outshift-common-dev`.

Then, run the following command to deploy the root spire:

```bash
cd root
helm upgrade --install spire-crds spiffe/spire-crds -n spire-system --create-namespace
c```

Once deployed you should get the following pods up and running:

```bash
k get pods -n spire-system
NAME                                              READY   STATUS    RESTARTS   AGE
spiffe-csi-driver-downstream-fc9jh                2/2     Running   0          45m
spiffe-csi-driver-downstream-snrrp                2/2     Running   0          45m
spiffe-csi-driver-downstream-tbpgp                2/2     Running   0          45m
spiffe-csi-driver-upstream-9l228                  2/2     Running   0          45m
spiffe-csi-driver-upstream-p8vxc                  2/2     Running   0          45m
spiffe-csi-driver-upstream-r7795                  2/2     Running   0          45m
spiffe-oidc-discovery-provider-8659c984bb-npk4x   2/2     Running   0          45m
spire-agent-downstream-8x5k4                      1/1     Running   0          45m
spire-agent-downstream-dhsrm                      1/1     Running   0          45m
spire-agent-downstream-fddgf                      1/1     Running   0          45m
spire-agent-upstream-6lvwb                        1/1     Running   0          45m
spire-agent-upstream-6qwp2                        1/1     Running   0          45m
spire-agent-upstream-t8h5d                        1/1     Running   0          45m
spire-external-server-0                           2/2     Running   0          45m
spire-internal-server-0                           2/2     Running   0          45m
spire-root-server-0                               2/2     Running   0          45m
```

## On Prem

The deployment on-prem is managed by Argocd. Values are copied here for clarity.

The on-prem deployment needs to be configured with the trust bundle to access the upstream
spire server. To do that, you can run the following command to get the bundle:

```bash
kubectl -n spire-system get configmap spire-bundle-upstream -o jsonpath='{.data.bundle\.spiffe}'
```

And then you need to set it into the values of the chart, under the key `upstreamBundle`.

After this operation, we need to register the nodes of the on-prem cluster to the spire server.
Set the kubectl context to the root cluster and run the following command to create an entry
for the node 0 of the on-prem cluster:

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

Notice that we will run a single upstream agent in the on-prem cluster. If you want to run
multiple agents, you need to create an entry for each of them, changing the parentID and the spiffeID accordingly.
We configured the on-prem cluster to run exacrtly one agent on node 0, so we create only one entry with the parentID of that node.

You should see the entry created in the spire server:

```bash
kubectl exec -n spire-system \
	spire-external-server-0 \
	-c external-spire-server -- \
	./bin/spire-server entry show \
		-parentID spiffe://slim.dev.outshift.ai/nested/upstream/gls-cluster-0/tf-kubeflow-node-0 \
		-spiffeID spiffe://slim.dev.outshift.ai/ns/spire-system/sa/spire-internal-server
```

Finally you can generate a join token for this spiffe ID:

```bash
kubectl exec -n spire-system \
	spire-external-server-0 \
	-c external-spire-server -- \
	./bin/spire-server token generate \
	-spiffeID spiffe://slim.dev.outshift.ai/nested/upstream/gls-cluster-0/tf-kubeflow-node-0 \
	-ttl 3600
```

Note the token generated, we will need it to join the on-prem cluster to the spire server.

This is managed by ArgoCD, but in principle you could also switch the kubectl context to
the on-prem cluster and run the following command to deploy the spire agent:

```bash
cd nested
helm upgrade --install spire-crds spire-crds -n spire-system --create
helm upgrade --install spire . -n spire-system --create-namespace -f values.yaml
```

After the deployment, no pod will be in Running state, because the helm chart does not allo
to set the join token directly. To set the join token, you can run the following commands (don't forget to switch the kubectl context to the on-prem cluster before):

```bash
# Set the join token generated in the previous step
JOIN_TOKEN=XXXXX

# 1. Extract current config
CURRENT=$(kubectl -n spire-system get configmap spire-agent-upstream -o jsonpath='{.data.agent\.conf}')

# 2. Add join_token to the agent block
UPDATED=$(echo "$CURRENT" | jq --arg token "$JOIN_TOKEN" '.agent.join_token = $token')

# 3. Patch the ConfigMap
kubectl -n spire-system patch configmap spire-agent-upstream --type merge -p "{\"data\":{\"agent.conf\":$(echo "$UPDATED" | jq -Rs .)}}"
```

Wait a moment and the spire-agent-upstream will become healthy. Once it gets running, also the spire-internal-server
will become healthy, because it depends on the upstream agent node attestation.

If you see the spire-agent-upstream is not becoming healthy, try to delete the pod, it will be recreated and it will pick up the new join token.
