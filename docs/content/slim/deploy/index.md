---
icon: material/rocket-launch
---

# Deployment Overview

SLIM can be deployed in several topologies, from a single local binary to a multi-cluster Kubernetes setup with SPIRE-based mTLS across trust domains. This page helps you choose the right deployment mode for your situation.

## Deployment Modes

| Mode | Use case | High availability | SPIRE required |
|------|----------|--------------------|----------------|
| [Local](./local.md) | Single-machine deployments, local agents, quick setups | No | No |
| [Docker](./docker.md) | Containerised single-host deployments, CI, portable setups | Optional | No |
| [Kubernetes: Deployment](./kubernetes.md) | Cluster-scale deployments, production services | Yes | Optional |
| [Kubernetes: DaemonSet](./daemonset.md) | Node-affine routing — one SLIM pod per node | Yes | Optional |
| [Kubernetes: Multi-Cluster](./multicluster.md) | Cross-cluster agent communication | Yes | Yes |

## What Gets Deployed

A SLIM deployment consists of one or more of these components:

- **SLIM Data Plane node** — required in all deployments; handles message routing between applications
- **SLIM Controller** — optional for single-cluster; required for multi-cluster; manages route tables and node configuration
- **SLIM Channel Manager** — optional; required if you want operator-managed group channels
- **SPIRE** — optional for single-cluster; required for multi-cluster; provides cryptographic identity and mTLS

SDK applications connect to a SLIM node and communicate over the data plane. They do not connect directly to the Controller or Channel Manager.

## Choosing a Mode

**Local binary** — start with [Local](./local.md) when you want a SLIM node running on a single machine with no additional infrastructure. The fastest path is `slimctl slim start`. Suitable for local agent setups — for example, coding agents or automation tools running on a developer workstation that need to communicate with each other.

**Docker** — use [Docker](./docker.md) for containerised single-host deployments or when you want a portable, self-contained SLIM setup. Docker Compose makes it easy to bring up a SLIM node alongside your agent services with a single command.

**Kubernetes Deployment** — use the [Kubernetes Deployment](./kubernetes.md) when running agents at cluster scale. Start with the simple pattern for straightforward setups, then add the Controller and SPIRE for production-grade deployments.

**Node-affine workloads** — use the [DaemonSet](./daemonset.md) pattern when application pods should always route through a SLIM pod on the same Kubernetes node. Kubernetes `internalTrafficPolicy: Local` ensures traffic stays node-local.

**Cross-cluster** — use the [Multi-Cluster](./multicluster.md) deployment when agents on different clusters need to communicate. This requires SPIRE federation between trust domains and either LoadBalancer services or an Ingress for inter-cluster connectivity.

## Related

- [Architecture](../architecture/index.md) — The four-layer SLIM stack
- [Getting Started](../slim-howto.md) — Install all components and run a first example
- [Authentication](../architecture/authentication.md) — TLS, mTLS, JWT, and SPIRE options
