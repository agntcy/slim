# SLIM Deployment Templates and Strategies

Deployment templates (or deployment strategies) describe various SLIM deployment patterns to support the most frequent usage scenarios.

These strategies primarily describe the deployment of SLIM from the operations point of view, that is, they focus on the SLIM infrastructure and topology.  

The following deployment strategies are described:

- [Naive Deployment Strategy](naive/naive_strategy.md)
- [Statefulset Deployment Strategy](statefulset/statefulset_strategy.md)
- [Daemonset Deployment Strategy](daemonset/daemonset_strategy.md)
- [Multi-cluster Deployment Strategy](multicluster/multi_cluster_strategy.md)

For each identified deployment strategy we provide the following:

- The description of the strategy, target audience and sample use cases
- A set of values files for the different components
- A set of tasks to execute the deployment
- Example client resources to demonstrate and test the strategy

## Considerations and Prerequisites

To mimic the real life deployments of SLIM, deployment strategies are executed in Kubernetes clusters using the publicly available SLIM Helm charts.

The following prerequisites must be met to execute the strategy examples locally via the provided taskfile:

- [Task](https://taskfile.dev/) available
- Docker engine available locally ([Docker Desktop](https://docs.docker.com/desktop/) or [Rancher Desktop](https://rancherdesktop.io/))
- [Kind](https://kind.sigs.k8s.io/) available on the local environment
- Access to public image repositories

## Example client image (`bindings-examples`)

Example workloads under [`client_apps/`](client_apps/) use the **slim-bindings** Python examples (e.g. `slim-bindings-p2p`, `slim-bindings-group`). Their manifests set a container `image` such as `agntcy/slim/bindings-examples:…`.

The local image is built from [`client_apps/Dockerfile`](client_apps/Dockerfile), which installs the **`slim-bindings[examples]`** package from PyPI (version pinned via `ARG SLIM_BINDINGS_VERSION`). To use a different bindings release, edit that `ARG` in the Dockerfile, then **remove the existing image** (e.g. `docker rmi agntcy/slim/bindings-examples:latest`) so the next build is not skipped when using `task apps:bindings-examples:prepare` (that task skips the build step while `docker image inspect` still succeeds).

### Automation via Task

From any strategy directory that includes [`client_apps/Taskfile.yaml`](client_apps/Taskfile.yaml), run:

```bash
task apps:bindings-examples:prepare
```

This builds `agntcy/slim/bindings-examples:latest` from [`client_apps/Dockerfile`](client_apps/Dockerfile) **only if** that image is not already present in the local Docker engine, then runs `kind load docker-image` for each cluster in `KIND_CLUSTER_NAMES` (default: `slim-cluster`). Override with:

`task apps:bindings-examples:prepare KIND_CLUSTER_NAMES=cluster-a,cluster-b`

Strategy tasks `test.sender:deploy` and `test.receiver:deploy` depend on `apps:bindings-examples:prepare` (single-cluster: default cluster name; multicluster: all three Kind clusters). Kind clusters must exist before running deploy.