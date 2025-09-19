## Description

The `naive` is the simplest deployment strategy. 

### Target audience
 It's mainly intended to be used by developers or users learning SLIM. 

### Details
A single plain SLIM node is started that exposes its (northbound and southbound) APIs, no authentication, secure communication (MLS) and group management is enabled.

## Example

The example deployment of the strategy can be acheved by executing the following tasks in a terminal (with the cursor at the folder where the Taskfile.yaml resides):

``` bash
# brings up the kind cluster
task cluster.up
```

``` bash
# deploys SLIM
task slim.deploy.naive
```

At this point, if the commands execute successfully there is a SLIM pod deployed to the cluster in the slim namespace.

``` bash
kubectl -n slim get po

NAME     READY   STATUS    RESTARTS   AGE
slim-0   1/1     Running   0          23h
```


``` bash
# deploys clients for testing the deployment (in the default namespace)
task slim.deploy.client-apps
```


### Commands to check the deployment

```bash
# check the SLIM logs
kubectl -n slim logs slim-0

# receiver client pod logs
kubectl -n logs alice

# sender client pod logs
kubectl -n logs bob







