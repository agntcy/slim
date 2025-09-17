# Echo Agent

## Run SLIM node
```shell
cd $(git rev-parse --show-toplevel)/data-plane/testing
```
```shell
task run:slim
```

## Run echo agent server
```shell
cd $(git rev-parse --show-toplevel)/data-plane/python/integrations/slima2a
```
```shell
uv run examples/echo_agent/server.py
```

## Run echo agent client
```shell
cd $(git rev-parse --show-toplevel)/data-plane/python/integrations/slima2a
```
```shell
uv run examples/echo_agent/client.py --text "hi, this is a text message"
```
