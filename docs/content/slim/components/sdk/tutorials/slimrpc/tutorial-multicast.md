# Tutorial: Multicast SLIMRPC

This tutorial shows how to fan out a single RPC call to multiple servers simultaneously and collect their responses. This is useful for broadcasting requests, scatter-gather queries, and any pattern where one caller addresses many workers at once.

## Prerequisites

- Completed [Connecting to SLIM](../tutorial-connect.md) and [Creating an App](../tutorial-app.md)
- Two or more running SLIMRPC servers — see [Serving a SLIMRPC Server](./tutorial-serve.md)
- Generated client stubs that include the group client (`TestGroupStub` / `NewTestGroupClient` / etc.)

## Create a Group Channel

A group channel wraps a SLIM group session that spans all the servers you want to call. Pass a list of remote server names instead of a single name.

=== "Rust"

    ```rust
    use slim_rpc::Channel;
    use slim_datapath::api::ProtoName;

    // server_names is a Vec<ProtoName> of all servers to fan out to
    let server_names = vec![
        ProtoName::from_strings(["myorg", "default", "server-1"]),
        ProtoName::from_strings(["myorg", "default", "server-2"]),
    ];

    // A group channel fans out every call to all members
    let channel = Channel::new_with_members_internal(
        app.clone(),
        server_names,
        true, // group = true for multicast
        Some(conn_id),
        tokio::runtime::Handle::current(),
    )?;
    ```

=== "Python"

    ```python
    import slim_bindings
    from types.example_pb2_slimrpc import TestGroupStub

    # server_names is a list of slim_bindings.Name objects
    server_names = [
        slim_bindings.Name("myorg", "default", "server-1"),
        slim_bindings.Name("myorg", "default", "server-2"),
    ]

    channel = slim_bindings.Channel.new_group_with_connection(app, server_names, conn_id)
    client = TestGroupStub(channel)
    ```

=== "Go"

    ```go
    import (
        slim "github.com/agntcy/slim-bindings-go"
        slim_rpc "github.com/agntcy/slim-bindings-go/slim_rpc"
    )

    server1, _ := slim.NameFromString("myorg/default/server-1")
    server2, _ := slim.NameFromString("myorg/default/server-2")
    serverNames := []*slim.Name{server1, server2}

    channel, err := slim_rpc.ChannelNewGroupWithConnection(app, serverNames, &connId)
    if err != nil {
        log.Fatal(err)
    }
    client := pb.NewTestGroupClient(channel)
    ```

=== "Java"

    ```java
    import io.agntcy.slim.bindings.slimrpc.Channel;
    import io.agntcy.slim.bindings.Name;
    import com.example_service.TestSlimrpc;
    import java.util.List;

    List<Name> serverNames = List.of(
        Name.fromString("myorg/default/server-1"),
        Name.fromString("myorg/default/server-2")
    );

    Channel channel = Channel.newGroupWithConnection(app, serverNames, connId);
    TestSlimrpc.TestGroupClientImpl client = new TestSlimrpc.TestGroupClientImpl(channel);
    ```

=== "Kotlin"

    ```kotlin
    import io.agntcy.slim.bindings.slimrpc.Channel
    import io.agntcy.slim.bindings.Name
    import com.example_service.TestSlimrpc

    val serverNames = listOf(
        Name.fromString("myorg/default/server-1"),
        Name.fromString("myorg/default/server-2")
    )

    val channel = Channel.newGroupWithConnection(app, serverNames, connId)
    val client = TestSlimrpc.TestGroupClientImpl(channel)
    ```

=== "Node.js"

    ```typescript
    import slimBindings from '@agntcy/slim-bindings';
    import { TestGroupClient } from './types/example_slimrpc.js';

    const serverNames = [
        new slimBindings.Name('myorg', 'default', 'server-1'),
        new slimBindings.Name('myorg', 'default', 'server-2'),
    ];
    const channel = slimBindings.Channel.newGroupWithConnection(app, serverNames, connId);
    const client = new TestGroupClient(channel);
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim.SlimRpc;
    using Agntcy.Slim;
    using ExampleService;

    var serverNames = new[]
    {
        SlimName.Parse("myorg/default/server-1"),
        SlimName.Parse("myorg/default/server-2"),
    };

    var channel = SlimRpcChannelFactory.CreateGroupChannel(app, serverNames, connId);
    var client = new TestGroupClient(channel);
    ```

## Send a Multicast Request

Call any RPC method on the group client. The call fans out to every server in the group. Responses arrive as a stream — one response per server, in arrival order.

Each response item carries both the response payload and the context identifying which server it came from.

=== "Rust"

    ```rust
    use example_service::{ExampleRequest, ExampleResponse};

    let request = ExampleRequest {
        example_string: "world".to_string(),
        example_integer: 42,
    };

    // Responses arrive as a stream — one per server, in arrival order
    let mut stream = channel
        .multicast_unary("example_service.Test", "ExampleUnaryUnary", request, None, None)
        .await?;

    while let Some(item) = stream.next().await {
        let (source, response): (ProtoName, ExampleResponse) = item?;
        println!("Response from {source:?}: {}", response.example_string);
    }
    ```

=== "Python"

    ```python
    from datetime import timedelta
    from types.example_pb2 import ExampleRequest

    request = ExampleRequest(example_string="world", example_integer=42)

    async for context, response in client.ExampleUnaryUnary(request, timeout=timedelta(seconds=5)):
        print(f"Response from {context.source}: {response.example_string}")
    ```

=== "Go"

    ```go
    import (
        "context"
        "fmt"
        "time"
    )

    ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
    defer cancel()

    request := &pb.ExampleRequest{ExampleString: "world", ExampleInteger: 42}
    stream, err := client.ExampleUnaryUnary(ctx, request)
    if err != nil {
        log.Fatal(err)
    }

    for {
        item, err := stream.Recv()
        if item == nil || err != nil {
            break
        }
        fmt.Printf("Response from %s: %s\n", item.Context.Source, item.Value.ExampleString)
    }
    ```

=== "Java"

    ```java
    import java.time.Duration;
    import com.example_service.ExampleRequest;
    import com.example_service.ExampleResponse;
    import io.agntcy.slim.bindings.slimrpc.MulticastResponseStream;
    import io.agntcy.slim.bindings.slimrpc.MulticastStreamMessage;
    import io.agntcy.slim.bindings.slimrpc.RpcMulticastItem;

    ExampleRequest request = ExampleRequest.newBuilder()
        .setExampleString("world")
        .setExampleInteger(42)
        .build();

    MulticastResponseStream<ExampleResponse> stream = client.ExampleUnaryUnary(request, Duration.ofSeconds(5), null);

    while (true) {
        MulticastStreamMessage msg = stream.next();
        if (msg instanceof MulticastStreamMessage.End) break;
        if (msg instanceof MulticastStreamMessage.Error error) {
            System.err.println("Error from server: " + error.error());
            continue;
        }
        if (msg instanceof MulticastStreamMessage.Data data) {
            RpcMulticastItem item = data.item();
            try {
                ExampleResponse resp = ExampleResponse.parseFrom(item.message());
                System.out.println("Response from " + item.context().source()
                    + ": " + resp.getExampleString());
            } catch (Exception e) { throw new RuntimeException(e); }
        }
    }
    ```

=== "Kotlin"

    ```kotlin
    import java.time.Duration
    import com.example_service.ExampleRequest
    import com.example_service.ExampleResponse
    import io.agntcy.slim.bindings.slimrpc.MulticastStreamMessage

    val request = ExampleRequest.newBuilder()
        .setExampleString("world")
        .setExampleInteger(42)
        .build()

    val stream = client.ExampleUnaryUnary(request, Duration.ofSeconds(5), null)

    while (true) {
        when (val msg = stream.next()) {
            is MulticastStreamMessage.End -> break
            is MulticastStreamMessage.Error -> System.err.println("Error from server: ${msg.error}")
            is MulticastStreamMessage.Data -> {
                val item = msg.item
                val resp = ExampleResponse.parseFrom(item.message)
                println("Response from ${item.context.source}: ${resp.exampleString}")
            }
        }
    }
    ```

=== "Node.js"

    ```typescript
    import { create } from '@bufbuild/protobuf';
    import { ExampleRequestSchema } from './types/example_pb.js';

    const request = create(ExampleRequestSchema, { exampleString: 'world', exampleInteger: 42n });

    for await (const { context, response } of client.ExampleUnaryUnary(request, 5000)) {
        console.log(`Response from ${context.source}: ${response.exampleString}`);
    }
    ```

=== ".NET"

    ```csharp
    using ExampleService;

    var request = new ExampleRequest { ExampleString = "world", ExampleInteger = 42 };

    await foreach (var item in client.ExampleUnaryUnaryAsync(request, timeout: TimeSpan.FromSeconds(5)))
    {
        Console.WriteLine($"Response from {item.Context}: {item.Value.ExampleString}");
    }
    ```

## Runnable Examples

- [Python group client example](https://github.com/agntcy/slim-bindings/blob/main/python/examples/slimrpc/simple/client_group.py)
- [Go group client example](https://github.com/agntcy/slim-bindings/blob/main/go/examples/slimrpc/simple/cmd/client_group/client_group.go)
- [Java/Kotlin group client example](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples/slimrpc/simple)
- [Node.js group client example](https://github.com/agntcy/slim-bindings/blob/main/node/examples/slimrpc/simple/client_group.ts)
- [.NET group client example](https://github.com/agntcy/slim-bindings/tree/main/dotnet/Slim.Examples.SlimRpc)

## Next Steps

- [SLIMRPC](../../slimrpc/index.md) — How multicast channels use SLIM group sessions under the hood
- [Using a SLIMRPC Server](./tutorial-client.md) — Point-to-point calls with all four streaming patterns
