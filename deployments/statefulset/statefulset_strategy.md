## Statefulset deployment strategy

## Description

Multiple SLIM nodes are deployed connected to a Controller.
Clients can connect to a k8s ClusterIP Service which load balances requests between SLIM nodes.

### Target audience


### Details


## Example

task cluster:up
task spire:deploy
task slim:contoller:deploy
task slim:deploy

k apply -f ../client_apps/with_spire/helper-config.yaml
k apply -f ../client_apps/with_spire/receiver-pod.yaml
k apply -f ../client_apps/with_spire/sender-pod.yaml


### Commands to check the deployment








