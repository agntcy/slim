## SLIM Usage templates

Usage templates describe various SLIM deployment patterns to help operations in supporting the most frequent scenarios.

We're using the existing Helm chart
The templats resume to the values files corresopnding to the scenarios

Node(s) are reached through a k8s service


We need an example client configuration that connects to nodes


Sections in each of the templates
- describe the use case, target audience (devs, experimenting)
- example values file
- describe what happens in the cluster, how cvan be teh node reached
- succession of commands (task file?)

### Single SLIM node Kubernetes

### One or more SLIM nodes with controller
- replicaset / statefulset - routes?!

- daemonset

### Multiple SLIM nodes with Controller in multiple Kubernetes clusters


## Application level usage templates
- TBD

- how do you connect to SLIM nodes in k8s w/ spire (ackuire jwt etc?)
