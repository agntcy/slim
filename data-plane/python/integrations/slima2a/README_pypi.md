# SLIMA2A

SLIMA2A is a native integration of A2A built on top of SLIM. 

## Server usage

```python
import srpc
from a2a.server.request_handlers import DefaultRequestHandler
from slima2a.handler import SRPCHandler
from slima2a.types.a2a_pb2_srpc import add_A2AServiceServicer_to_server

agent_executor = MyAgentExecutor()
request_handler = DefaultRequestHandler(
     agent_executor=agent_executor, task_store=InMemoryTaskStore()
)

servicer = SRPCHandler(agent_card, request_handler)

server = srpc.Server(
    local="agntcy/demo/server",
    slim={
        "endpoint": "http://localhost:46357",
        "tls": {
            "insecure": True,
            },
        },
        shared_secret="secret",
    )

a2a_pb2_srpc.add_A2AServiceServicer_to_server(
        servicer
        server,
    )

await server.start()
```

## Client Usage

```python
from srpc import SRPCChannel
from a2a.client import ClientFactory, minimal_agent_card
from slima2a.client_transport import SRPCTransport, ClientConfig

def channel_factory(topic) -> SRPCChannel:
    channel = srpc.Channel(
        local="agntcy/demo/client",
        remote="agntcy/demo/server",
        slim={
            "endpoint": "http://localhost:46357",
            "tls": {
                "insecure": True,
                },
            },
        shared_secret="secret",
    )
    return channel

client_config = ClientConfig(
    supported_transports=["JSONRPC", "srpc"],
    streaming=args.stream,
    httpx_client=httpx_client,
    srpc_channel_factory=channel_factory,
)
client_factory = ClientFactory(client_config)
client_factory.register("srpc", SRPCTransport.create)

ac = minimal_agent_card("agntcy/demo/server", ["srpc"])
client = factory.create(ac)

try:
    response = client.send_message(...)
except srpc.SRPCResponseError as e:
    ...
```
