# Tutorial: Multicast SLIMRPC

This tutorial shows how to fan out a single RPC call to multiple servers simultaneously and collect their responses. This is useful for broadcasting requests, scatter-gather queries, and any pattern where one caller addresses many workers at once.

## Prerequisites

- Completed [Connecting to SLIM](../tutorial-connect.md) and [Creating an App](../tutorial-app.md)
- Two or more running SLIMRPC servers — see [Serving a SLIMRPC Server](./tutorial-serve.md)
- Generated client stubs that include the group client (`TestGroupStub` / `NewTestGroupClient` / etc.)

## Create a Group Channel

A group channel wraps a SLIM group session that spans all the servers you want to call. Pass a list of remote server names instead of a single name.

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
    import slim "github.com/agntcy/slim-bindings-go"

    server1, _ := slim.NameFromString("myorg/default/server-1")
    server2, _ := slim.NameFromString("myorg/default/server-2")
    serverNames := []slim.Name{server1, server2}

    channel, err := slim.ChannelNewGroupWithConnection(app, serverNames, &connId)
    if err != nil {
        log.Fatal(err)
    }
    client := pb.NewTestGroupClient(channel)
    ```

=== "Java"

    ```java
    import io.agntcy.slim.bindings.Channel;
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
    import io.agntcy.slim.bindings.Channel
    import io.agntcy.slim.bindings.Name
    import com.example_service.TestSlimrpc

    val serverNames = listOf(
        Name.fromString("myorg/default/server-1"),
        Name.fromString("myorg/default/server-2")
    )

    val channel = Channel.newGroupWithConnection(app, serverNames, connId)
    val client = TestSlimrpc.TestGroupClientImpl(channel)
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim.Rpc;
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

=== "Python"

    ```python
    from datetime import timedelta
    from types.example_pb2 import ExampleRequest

    request = ExampleRequest(example_string="world", example_integer=42)

    async for context, response in client.ExampleUnaryUnary(request, timeout=timedelta(seconds=5)):
        print(f"Response from {context.source_name}: {response.example_string}")
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
        fmt.Printf("Response from %s: %s\n", item.Context.SourceName, item.Value.ExampleString)
    }
    ```

=== "Java"

    ```java
    import com.example_service.ExampleRequest;
    import io.agntcy.slim.bindings.slimrpc.MulticastResponseStream;

    ExampleRequest request = ExampleRequest.newBuilder()
        .setExampleString("world")
        .setExampleInteger(42)
        .build();

    MulticastResponseStream<com.example_service.ExampleResponse> stream =
        client.ExampleUnaryUnary(request).get();

    while (stream.hasNext()) {
        var item = stream.next();
        System.out.println("Response from " + item.context().sourceName()
            + ": " + item.value().getExampleString());
    }
    ```

=== "Kotlin"

    ```kotlin
    import com.example_service.ExampleRequest

    val request = ExampleRequest.newBuilder()
        .setExampleString("world")
        .setExampleInteger(42)
        .build()

    client.ExampleUnaryUnary(request).collect { item ->
        println("Response from ${item.context.sourceName}: ${item.value.exampleString}")
    }
    ```

=== ".NET"

    ```csharp
    using ExampleService;

    var request = new ExampleRequest { ExampleString = "world", ExampleInteger = 42 };

    await foreach (var item in client.ExampleUnaryUnary(request, timeout: TimeSpan.FromSeconds(5)))
    {
        Console.WriteLine($"Response from {item.Context.SourceName}: {item.Value.ExampleString}");
    }
    ```

## Runnable Examples

- [Python group client example](https://github.com/agntcy/slim-bindings/blob/main/python/examples/slimrpc/simple/client_group.py)
- [Go group client example](https://github.com/agntcy/slim-bindings/blob/main/go/examples/slimrpc/simple/cmd/client_group/client_group.go)
- [Java/Kotlin group client example](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples/slimrpc/simple)
- [.NET group client example](https://github.com/agntcy/slim-bindings/tree/main/dotnet/Slim.Examples.SlimRpc)

## Next Steps

- [SLIMRPC](./index.md) — How multicast channels use SLIM group sessions under the hood
- [Using a SLIMRPC Server](./tutorial-client.md) — Point-to-point calls with all four streaming patterns
