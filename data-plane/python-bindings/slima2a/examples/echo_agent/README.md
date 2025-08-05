# Read me

## echo agent server

```shell
cd ${SLIM_REPO}/data-plane/python-bindings/slima2a
```
```shell
uv run examples/echo_agent/server.py
```

## echo agent client

```shell
cd ${SLIM_REPO}/data-plane/python-bindings/slima2a
```
```shell
uv run examples/echo_agent/client.py --text "hi, this is a text message" [--stream] [--log-level=INFO]
```