## Multi cluster deployment strategy

This strategy demonstrates how SLIM nodes in different clusters can be connected to each other and clients connecting to any of the nodes can reach each other.

## Description

SLIM nodes are deployed as StatefulSet on two separate clusters, exposing an Ingress endpoint.
They are configured with this external endpoint.
The Controller is deployed on of the clusters, it's endpoint is exposed via an Ingress.
Sender client is deployed on cluster A and connected to SLIM service endpoint in local cluster, while receiver client is deployed to the other cluster.

Two clusters are federated with Spire.
Routes are created by Controller upon new subscriptions. SLIM nodes are connected to each other and Controller using Spire Federation.
Clients are also running on different clusters connecting with MTLS using Spire Federation.

### Target audience


### Details


## Setup steps in detail


Setup cluster-a and cluster-b on Kind and start Kind LoadBalancer Provider.

sudo task multi-cluster:up

Deploy Spire (federated mode enabled) on cluster-a & cluster-b:

task spire:deploy

Add spire.cluster-a.org & spire.cluster-b.org to /etc/hosts.

Create cluster federation resources on both clusters:

task spire:federation:deploy

Deploy controller on cluster-a:

task slim:contoller:deploy

Add slim-control.cluster-a.org to /etc/hosts.

Deploy SLIM on cluster-a & cluster-b:

task slim:deploy

Add slim.cluster-a.org & slim.cluster-b.org to /etc/hosts.

Deploy Alice (receiver) from with_spire on cluster-a then Bob (sender) on cluster-b.

k apply -f ../client_apps/with_spire/alice-pod.yaml
k apply -f ../client_apps/with_spire/bob-pod.yaml

### Commands to check the deployment










