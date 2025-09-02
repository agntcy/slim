# SLIMA2A

# Server Usage

```
from a2a.server.request_handlers import DefaultRequestHandler

agent_executor = MyAgentExecutor()
request_handler = DefaultRequestHandler(
     agent_executor=agent_executor, task_store=InMemoryTaskStore()
)

servicer = SRPCHandler(agent_card, request_handler)

server = srpc.server()
a2a_pb2_srpc.add_A2AServiceServicer_to_server(
        servicer
        server,
    )

await server.start()
```

# Client Usage

```
from srpc import SRPCChannel
from a2a.client import ClientFactory, minimal_agent_card
from slima2a.client_transport import SRPCTransport, ClientConfig

def channel_factory(topic) -> SRPCChannel:
    channel = SRPCChannel(
        local=local,
        slim=slim,
        enable_opentelemetry=enable_opentelemetry,
        shared_secret=shared_secret,
    )
    await channel.connect(topic)
    return channel

clientConfig = ClientConfig(srpc_channel_factor=channel_factor)

factory = ClientFactory(clientConfig)
factory.register('srpc', SRPCTransport.create)
ac = minimal_agent_card(topic, ["srpc"])
client = factory.create(ac)

try:
    response = client.send_message(...)
except srpc.SRPCResponseError as e:
    ...
```
