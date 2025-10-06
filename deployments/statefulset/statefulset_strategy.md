# Statefulset Deployment Strategy

## Description

The statefulset deployment strategy provides a more robust approach to deploying SLIM in a Kubernetes cluster, specifically designed for scenarios requiring high availability and ordered deployment patterns. This strategy deploys SLIM itself as a StatefulSet rather than a standard Deployment.

**Target Audience:**
- Production environments requiring high availability
- Applications needing ordered scaling and updates
- Deployments requiring predictable pod management

**Use Cases:**
- Production SLIM deployments with high availability
- Applications with complex initialization sequences
- Deployments needing predictable pod naming and ordering
- Environments requiring reliable service continuity

## Details

Multiple SLIM nodes are deployed connected to a Controller. Clients can connect to a k8s ClusterIP Service which load balances requests between SLIM nodes. Routes are created by Controller upon new subscriptions. SLIM nodes are connected to each other and Controller using Spire. Clients are also running on the same cluster connecting with MTLS using Spire.

The statefulset deployment strategy deploys SLIM components with enhanced configuration:
- StatefulSet-based deployment for SLIM pods
- Ordered pod management for reliable scaling
- Enhanced configuration through dedicated values file

This approach deploys SLIM as a StatefulSet, which provides guarantees around ordered deployment and scaling, supporting high availability scenarios. The deployment is driven by the `statefulset-values.yaml` file, which contains specific configuration parameters optimized for stateful workloads.

**Key Features:**
- Ordered deployment and scaling for high availability
- Predictable pod management
- Enhanced SLIM configuration options

![SLIM StatefulSet Deployment Diagram](img/slim_statefulset.svg)

## Usage

Follow these steps to deploy SLIM using the statefulset deployment strategy:

### 1. Set up the Kubernetes cluster
```bash
task templates:cluster:up
```

### 2. Deploy Spire
```bash
task templates:spire:deploy
```

### 3. Deploy SLIM controller
```bash
task templates:slim:controller:deploy
```

### 4. Deploy SLIM as StatefulSet
```bash
task slim:deploy
```

### 4. Verify the deployment
```bash
kubectl get statefulsets -n slim
kubectl get pods -n slim
```

### 5. View SLIM logs
```bash
task slim:show-logs
```

### 6. (Optional) Deploy sample client applications for testing
```bash
task apps:spire:receiver:deploy
```

### 6. (Optional) Deploy sample client applications for testing
```bash
task apps:spire:sender:deploy
```

### 7. Clean up when done
```bash
task slim:delete
task templates:cluster:down
```

**Note:** The statefulset strategy uses the `statefulset-values.yaml` file for Helm chart configuration. This values file contains specific settings for StatefulSet deployment, including SLIM-specific parameters and enhanced resource definitions. Review and customize this file according to your SLIM requirements.