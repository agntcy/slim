## Daemonset deployment strategy

## Description

SLIM nodes are deployed as DaemonSet on each node there's a SLIM instance deployed.
Sender client will connect to a k8s ClusterIP Service which load balances requests between SLIM nodes, while receiver clients will be deployed as a DaemonSet connecting to local SLIM instances via a ClusterIP Service with internalTrafficPolicy = Local so they will connect to the local SLIM instance running on the same node.

### Target audience


### Details


## Example



### Commands to check the deployment








