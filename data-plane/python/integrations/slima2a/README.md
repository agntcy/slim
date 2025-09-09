# SLIMA2A

SLIMA2A is a native integration of A2A built on top of SLIM. It utilizes SRPC and the
SRPC compiler to compile A2A protobuf file and generate the necessary code to enable 
A2A functionality on SLIM.

# What is SRCP and SRCP compiler

SRPC (SLIM Remote Procedure Call) is a framework that enables Protocol Buffers (protobuf)
Remote Procedure Calls (RPC) over SLIM. This is similar to gRPC, which uses HTTP/2 as 
its transport layer for protobuf-based RPC. [More information can be found here: TODO]

To compile a protobuf file and generate the clients and service stub you can use
the SRPC compiler (see ... TODO). This works in a similar way to the protoc compiler.

For SLIM A2A we compiled the [a2a.proto](https://github.com/a2aproject/A2A/blob/main/specification/grpc/a2a.proto)
file using the SRPC compiler. The generated code is located at 
`slima2a/types/a2a_pb2_srpc.py`.

# How to use SLIM A2A

Use SLIM A2A is very similar to use the standard A2A implementation. As a
reference example here we use the
[travel planner agent](https://github.com/a2aproject/a2a-samples/tree/main/samples/python/agents/travel_planner_agent)
available on the A2A samples repo. The version adapted to use SLIM A2A can be found in
```slima2a/examples/travel_planner_agent```. In the following section, we highlight 
and explain the key differences between the standard and SLIM A2A implementations.

## Travel Planner: Server

The implementation of the server is available at ```slima2a/examples/travel_planner_agent/server```.
Below, we highlight the main differences from the original implementation in the A2A repository.

1. Import the SRPC package
```python
import srpc
```

2. create the SRPCHandler. Notice that the definitions for `AgentCard` and 
`DefaultRequestHandler` remain unchanged from the original A2A example
```python
    agent_card = AgentCard(
        name="travel planner Agent",
        description="travel planner",
        url="http://localhost:10001/",
        version="1.0.0",
        default_input_modes=["text"],
        default_output_modes=["text"],
        capabilities=AgentCapabilities(streaming=True),
        skills=[skill],
    )

    request_handler = DefaultRequestHandler(
        agent_executor=TravelPlannerAgentExecutor(),
        task_store=InMemoryTaskStore(),
    )

    servicer = SRPCHandler(agent_card, request_handler)
```
3. Setup the srcp.Server. This is the only place where 
you need to setup few parameters ther are specific to SLIM
```python
    server = srpc.Server(
        local="agntcy/demo/travel_planner_agent",
        slim={
            "endpoint": "http://localhost:46357",
            "tls": {
                "insecure": True,
            },
        },
        shared_secret="secret",
    )
```
	•	local: Name of the local application.
	•	slim: Dictionary specifying how to connect to the SLIM node.
	•	shared_secret: Used to set up MLS (Message Layer Security).
For more information about these settings, see the SRPC README ([TODO: add reference]).

4. Register the Sevice
```python
    add_A2AServiceServicer_to_server(
        servicer,
        server,
    )
```

Your A2A server is now ready to run on SLIM.


## Travel Planner: Client
The client code is available at ```slima2a/examples/travel_planner_agent/client```.

1. Create a channel. This requires a configuration that is similar to the server
```python
    def channel_factory(topic: str) -> srpc.Channel:
        channel = srpc.Channel(
            local="agntcy/demo/client",
            remote=topic,
            slim={
                "endpoint": "http://localhost:46357",
                "tls": {
                    "insecure": True,
                },
            },
            shared_secret="secret",
        )
        return channel
```

2. Specify SRCP in the supported transports.

```python
    client_config = ClientConfig(
        supported_transports=["JSONRPC", "srpc"],
        streaming=True,
        httpx_client=httpx_client,
        srpc_channel_factory=channel_factory,
    )
    client_factory = ClientFactory(client_config)
    client_factory.register("srpc", SRPCTransport.create)
    agent_card = minimal_agent_card("agntcy/demo/travel_planner_agent", ["srpc"])
    client = client_factory.create(card=agent_card)
```

## How to run
TODO

<!--
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
--->
