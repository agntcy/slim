## Multi cluster deployment strategy

This strategy demonstrates how SLIM nodes in different clusters can be connected to each other and clients connecting to any of the nodes can reach each other.

## Description

SLIM nodes are deployed as StatefulSet on two separate clusters, exposing an Ingress endpoint.
They are configured with this external endpoint.
The Controller is deployed on of the clusters, it's endpoint is exposed via an Ingress.
Sender client is deployed on cluster A and connected to SLIM service endpoint in local cluster, while receiver client is deployed to the other cluster.

### Target audience


### Details


## Example

task multi-cluster:up
task spire:federation:deploy
task slim:contoller:deploy
task slim:deploy

deploy receiver from with_sprei on one cluster then sender on other one.

### Commands to check the deployment








