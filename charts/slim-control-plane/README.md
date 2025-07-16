# SLIM Control Plane Helm Chart

This Helm chart deploys the SLIM Control Plane components to a Kubernetes cluster.

## Components

- **Token Service**: Manages authentication tokens for SLIM agents

## Prerequisites

- Kubernetes 1.19+
- Helm 3.2.0+
- Token Service Docker image (must be built separately)

## Building the Token Service Image

The token-service image needs to be built separately before deploying this chart:

```bash
# From the repository root
docker build -t ghcr.io/agntcy/slim/token-service:latest -f control-plane/token-service/Dockerfile .
```

## Installing the Chart

To install the chart with the release name `my-control-plane`:

```bash
helm install my-control-plane charts/slim-control-plane/
```

## Uninstalling the Chart

To uninstall/delete the `my-control-plane` deployment:

```bash
helm delete my-control-plane
```

## Configuration

The following table lists the configurable parameters and their default values.

| Parameter | Description | Default |
|-----------|-------------|---------|
| `tokenService.replicaCount` | Number of token service replicas | `1` |
| `tokenService.image.repository` | Token service image repository | `ghcr.io/agntcy/slim/token-service` |
| `tokenService.image.tag` | Token service image tag | `""` (uses chart appVersion) |
| `tokenService.image.pullPolicy` | Image pull policy | `IfNotPresent` |
| `tokenService.service.type` | Service type | `ClusterIP` |
| `tokenService.service.port` | Service port | `8080` |
| `serviceAccount.create` | Create service account | `true` |
| `serviceAccount.name` | Service account name | `slim-control-plane` |

## Usage with SLIM Data Plane

This control plane chart is designed to work alongside the main SLIM chart. Deploy both charts to have a complete SLIM setup:

```bash
# Deploy control plane
helm install slim-control-plane charts/slim-control-plane/

# Deploy data plane
helm install slim charts/slim/
```

The SLIM data plane chart has been updated to expose the controller API port (46358) to communicate with control plane components.