# SLIM Deployment Templates and Strategies

Deployment templates (or deployment strategies) describe various SLIM deployment patterns to support the most frequent usage scenarios.

These strategies primarily describe the deployment of SLIM from the operations point of view, that is, they focus on the SLIM infrastructure and topology.  

The following deployment strategies are described:

- [Naive Deployment Strategy](naive/naive_strategy.md)
- [Statefulset Deployment Strategy](statefulset/statefulset_strategy.md)
- [Daemonset Deployment Strategy](daemonset/daemonset_strategy.md)
- [Multi-cluster Deployment Strategy](multicluster/multi_cluster_strategy.md)

For each identified deployment strategy we provide the followings:

- The description of the strategy, target audience and sample use cases
- A set of values files for the different components
- A set of tasks to execute the deployment
- Example client resources to demonstrate and test the strategy

## Considerations and Prerequisites

To mimic the real life deployments of SLIM, deployment strategies are executed in Kubernetes clusters using the publicly available SLIM Helm charts.

In order for the strategy examples to be executed locally through the provided taskfile, the following prerequisites are needed:

- [Task](https://taskfile.dev/) available
- Docker engine available locally ([Docker Desktop](https://docs.docker.com/desktop/) or [Rancher Desktop](https://rancherdesktop.io/))
- [Kind](https://kind.sigs.k8s.io/) available on the local environment
- Access to public image repositories
