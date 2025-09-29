## Statefulset deployment strategy

## Description

Multiple SLIM nodes are deployed connected to a Controller.
Clients can connect to a k8s ClusterIP Service which load balances requests between SLIM nodes.
Routes are created by Controller upon new subscriptions. SLIM nodes are connected to each other and Controller using Spire.
Clients are also running on the same cluster connecting with MTLS using Spire.


### Target audience


### Details


## Setup steps in detail

task cluster:up
task spire:deploy
task slim:contoller:deploy
task slim:deploy

Deploy receiver from with_spire on one cluster then sender on other one.

k apply -f ../client_apps/with_spire/helper-config.yaml
k apply -f ../client_apps/with_spire/receiver-pod.yaml
k apply -f ../client_apps/with_spire/sender-pod.yaml


### Commands to check the deployment








