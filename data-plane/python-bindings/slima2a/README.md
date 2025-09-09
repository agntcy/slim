# SLIMA2A

SLIMA2A is a native integration of A2A on top of SLIM. It uses SRPC and SRPC compiler to 
compiler the A2A protobuf file and generate the necessary code to run A2A on top of SLIM.

# What is SRCP and SRCP compiler

SRPC, or SLIM Remote Procedure Call, is a mechanism designed to enable Protocol
Buffers (protobuf) RPC over SLIM (Secure Low-latency Inter-process Messaging). 
This is analogous to gRPC, which leverages HTTP/2 as its underlying transport
layer for protobuf RPC. You can find more info on this in TODO

To compile a protobuf file and generate the clients and service stub you can use
the SRPC compiler (see ... TODO). This works in a similer way to the protoc compiler.

For SLIM A2A we compiled the [a2a.proto](https://github.com/a2aproject/A2A/blob/main/specification/grpc/a2a.proto)
file using the SRPC compiler. The generated code can be found in 
```slima2a/types/a2a_pb2_srpc.py```.

# How to use SLIM A2A

Use SLIM A2A is very similar to use the standard A2A implementation. As a
reference example here we start from the
[travel planner agent](https://github.com/a2aproject/a2a-samples/tree/main/samples/python/agents/travel_planner_agent)
available on the A2A samples repo. The modified version that uses SLIM A2A is
available in ```slima2a/examples/travel_planner_agent```. In the following 
session we highlight and explane the differences between the two implementations


## Travel Planner: Server

TODO compare __main__.py with server

## Travel Planner: Client

TODO compare loop_client.py with client

## How to run

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

## Client Usage

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
