## SLIM deployment templates / strategies

Deployment templates (or deployment strategies) describe various SLIM deployment patterns to to support the most frequent usage scenarios.

These strategies primarily describe the deployment of SLIM from the operations point of view, that is, they focus on the SLIM infrastructure and topology.  

For each identified deployment strategy we provide the followings
- the description of the strategy / recommended use cases
- a set of values files for the different components
- a set of tasks to execute the deployment
- example client resources to demonstrate / test the startegy

## Considerations / prerequisites

To mimic the real life deployments of SLIM, deployment strategies are executed in Kubernetes clusters using the publicly available SLIM Helm charts. 

In order for the strategy examples to be executed locally through the provided taskfile, the following prerequisites are needed:

- [task](https://taskfile.dev/) available
- docker engine available locally (Docker Desktop or Rancher Desktop)
- kind available on the local environment
- access to public image repositories

The templats resume to the values files corresopnding to the scenarios

Node(s) are reached through a k8s service


We need an example client configuration that connects to nodes

## Table of Contents

- SLIM deployment strategies
  - [Naive deployment strategy](naive/naive_strategy.md)
  - [Statefulset deployment strategy](statefulset/statefulset_strategy.md)
  - [Daemonset deployment strategy](daemonset/daemonset_strategy.md)

