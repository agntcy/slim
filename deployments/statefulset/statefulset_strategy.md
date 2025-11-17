# Statefulset Deployment Strategy

## Description

The statefulset deployment strategy provides a more robust approach to deploying SLIM in a Kubernetes cluster, specifically designed for scenarios requiring high availability and ordered deployment patterns.

**Target Audience:**

- Production environments requiring high availability.
- Applications needing ordered scaling and updates.
- Deployments requiring predictable pod management.

**Use Cases:**

- Production SLIM deployments with high availability.
- Applications with complex initialization sequences.
- Deployments needing predictable pod naming and ordering.
- Environments requiring reliable service continuity.

## Details

This approach deploys SLIM nodes as a StatefulSet which guarantees ordered deployment and scaling, supporting high availability scenarios. All nodes are connected to one single Controller using MTLS. Clients are connecting to a `ClusterIP` service which load balances connections between SLIM nodes. Since SLIM keeps the connection open, all communication goes to the same server.

Forwarding routes between the nodes are managed automatically by Controller on-demand upon new subscriptions. SLIM nodes are connected directly via their pod IP.

All connections between SLIM nodes and SLIM nodes and Controller are set up using MTLS. Clients are also deployed on the cluster connecting with MTLS, but in addition messages are encrypted using MLS authenticating with JWT tokens.

SLIM leverages [SPIRE](https://spiffe.io/docs/latest/spire-about/) to resolve trust between different components of the system: SLIM nodes, SLIM clients, Controller. SPIRE establishes a zero-trust identity framework where every process receives an SVID along with automatically rotated certificates and JWT tokens for mutual authentication.

The deployment is driven by the `statefulset-values.yaml` file, which contains specific configuration parameters optimized for stateful workloads.

**Key Features:**

- Ordered deployment and scaling for high availability.
- Predictable pod management.
- Enhanced SLIM configuration options.

![SLIM StatefulSet Deployment Diagram](img/slim_statefulset.svg)

## Usage

Follow these steps to deploy SLIM using the statefulset deployment strategy:

### 1. Set up the Kubernetes cluster

```bash
task cluster:up
```

### 2. Deploy Spire

```bash
task spire:deploy
```

<details>
<summary>More Details on Spire</summary>

This step deploys a Spire server, agent and controller on the cluster.

The Spire controller automatically creates SVIDs, certificates, JWT tokens and registers the created *SPIFFEID* for each running pod driven by `ClusterSPIFFEID` custom resource.

Here's an example of the default `ClusterSPIFFEID` which applies to all pods unless a custom one is defined:

```yaml
  apiVersion: spire.spiffe.io/v1alpha1
  kind: ClusterSPIFFEID
  name: spire-spire-default
  spec:
      className: spire-spire
      fallback: true
      hint: default
      namespaceSelector:
          matchExpressions:
          - key: kubernetes.io/metadata.name
          operator: NotIn
          values:
          - spire
          - spire-server
          - spire-system
      spiffeIDTemplate: spiffe://{{ .TrustDomain }}/ns/{{ .PodMeta.Namespace }}/sa/{{
          .PodSpec.ServiceAccountName }}
```

The Spire agent exposes a local API endpoint on each node, used by applications to fetch certificates and certificate bundles. Applications may support Spire natively (Controller) or they run [Spiffe helper](https://github.com/spiffe/spiffe-helper) as a side-car, which takes care or fetching and rotating certificates and tokens. (SLIM nodes and clients)

For troubleshooting connection problems, use the following command to list created entries:

```bash
kubectl exec -n spire spire-server-0 -- /opt/spire/bin/spire-server entry
```

Find out more on [Spire on Kubernetes](https://spiffe.io/docs/latest/try/getting-started-k8s/) and [ClusterSPIFFEID](https://github.com/spiffe/spire-controller-manager/blob/main/docs/clusterspiffeid-crd.md) resource.

</details>

### 3. Deploy SLIM controller chart

```bash
task slim:controller:deploy
```

<details>
<summary>More Details on Controller configuration</summary>

This step deploys the SLIM Controller Southbound API configured with MTLS. The Controller supports MTLS through SPIRE, which just needs to be enabled and the `socketPath` configured:

```yaml
  config:
      northbound:
          httpHost: 0.0.0.0
          httpPort: "{{ .Values.service.north.port }}"

      southbound:
          httpHost: 0.0.0.0
          httpPort: "{{ .Values.service.south.port }}"    
          tls:
              useSpiffe: true
          spire:
              socketPath: "unix:///run/spire/agent-sockets/api.sock"
```

See [SLIM Controller Helm chart values](../controller-values.yaml) for more information.

</details>


### 4. Deploy SLIM nodes from helm chart

```bash
task slim:deploy
```

<details>
<summary>More Details on SLIM node configuration</summary>

This step deploys three replicas of SLIM servers and a `ClusterIP` service.

In each SLIM pod, there is a `spire-helper` container running fetching generated X509 certificates, keys, and certificate bundles to the configured path. SLIM nodes must be configured to use MTLS using the same path.

```yaml
  services:
    slim/0:
      node_id: ${env:SLIM_SVC_ID}
      dataplane:
        servers:
          - endpoint: "0.0.0.0:{{ .Values.slim.service.data.port }}"
            tls:
              #insecure: true
              insecure_skip_verify: false
              cert_file: "/svids/tls.crt"
              key_file: "/svids/tls.key"
              ca_file: "/svids/svid_bundle.pem"                

        clients: []
      controller:
        clients:
          - endpoint: "https://slim-control:50052"
            tls:
              #insecure: true
              insecure_skip_verify: false
              cert_file: "/svids/tls.crt"
              key_file: "/svids/tls.key"
              ca_file: "/svids/svid_bundle.pem"
```

> **Note:** `node_id` should be a unique ID within the cluster, since this is used by Controller to identify SLIM server.

See [SLIM Helm chart values](statefulset-values.yaml) for more information.

</details>

### 5. Verify nodes are connected

At this point you can check that all nodes are connected, in Controller logs:

```bash
kubectl logs -n slim deployment/slim-control | grep "Registering node with ID"
```

### 6. Deploy sample client applications for testing

The sample applications demonstrate in-cluster communication with centralized control:

- Alice (receiver) subscribes to messages and replies to received messages.
- Bob (sender) creates a new MLS session publishes messages and waits for reply.

Each client uses SPIRE federation for authentication, running spiffe-helper as a side-car.
  
The centralized Controller automatically creates routes when Alice subscribes, enabling Bob's messages to reach Alice through the controller coordination.
  
```bash
# Deploy receiver (Alice)
task test:receiver:deploy
# Deploy sender (Bob)
task test:sender:deploy
```

Check client logs:

```bash
kubectl logs alice client
kubectl logs bob client
```

You should see 10 messages sent and received.

<details>
<summary>Troubleshooting tips</summary>

Check the SLIM node logs on each cluster:

```bash
kubectl logs -n slim slim-0 slim
kubectl logs -n slim slim-1 slim
```

In case of connection problems, check the following:

* List registration entries on each cluster:

    ```bash
    kubectl exec -n spire spire-server-0 -- /opt/spire/bin/spire-server entry show
    ```

    There should be an entry for Controller, one entry for each SLIM node.

* Check `spiffe-helper` side-car logs in SLIM nodes and client apps:

    ```bash
    kubectl logs -n slim slim-0 spiffe-helper
    kubectl logs -n slim slim-1 spiffe-helper
    kubectl logs alice spiffe-helper
    kubectl logs bob spiffe-helper
    ```

</details>

### 7. Clean up when done

```bash
task cluster:down
```

> **Note:** The statefulset strategy uses the `statefulset-values.yaml` file for Helm chart configuration. This values file contains specific settings for StatefulSet deployment, including SLIM-specific parameters and enhanced resource definitions. Review and customize this file according to your SLIM requirements.
