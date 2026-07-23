# Tutorial: Using a SLIMRPC Server

This tutorial shows how to connect to a running SLIMRPC server, create a channel, and call each of the four RPC patterns: unary-unary, unary-stream, stream-unary, and stream-stream.

## Prerequisites

- Completed [Connecting to SLIM](../tutorial-connect.md) and [Creating an App](../tutorial-app.md) — you need the `app` and `conn_id` objects
- A running SLIMRPC server — see [Serving a SLIMRPC Server](./tutorial-serve.md)
- Generated client stubs for your service — see Step 2 of the serving tutorial

## Create a SLIMRPC Channel

A SLIMRPC channel wraps the SLIM session layer. Pass it the remote server's SLIM name and the connection ID from `connect_async`.

=== "Python"

    ```python
    import slim_bindings
    from types.example_pb2_slimrpc import TestStub

    # app, conn_id, and remote_name come from the prerequisite tutorials
    channel = slim_bindings.Channel.new_with_connection(app, remote_name, conn_id)
    client = TestStub(channel)
    ```

=== "Go"

    ```go
    import (
        slim "github.com/agntcy/slim-bindings-go"
        pb "example/types"
    )

    // app, connId, and remoteName come from the prerequisite tutorials
    channel := slim.ChannelNewWithConnection(app, remoteName, &connId)
    client := pb.NewTestClient(channel)
    ```

=== "Java"

    ```java
    import io.agntcy.slim.bindings.Channel;
    import com.example_service.TestSlimrpc;

    // app, connId, and remoteName come from the prerequisite tutorials
    Channel channel = Channel.newWithConnection(app, remoteName, connId);
    TestSlimrpc.TestClientImpl client = new TestSlimrpc.TestClientImpl(channel);
    ```

=== "Kotlin"

    ```kotlin
    import io.agntcy.slim.bindings.Channel
    import com.example_service.TestSlimrpc

    // app, connId, and remoteName come from the prerequisite tutorials
    val channel = Channel.newWithConnection(app, remoteName, connId)
    val client = TestSlimrpc.TestClientImpl(channel)
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim.Rpc;
    using ExampleService;

    // app, connId, and remoteName come from the prerequisite tutorials
    var channel = SlimRpcChannelFactory.CreateChannel(app, remoteName, connId);
    var client = new TestClient(channel);
    ```

=== "Rust"

    ```rust
    use slim_rpc::Channel;

    // app, conn_id, and remote_name come from the prerequisite tutorials
    let channel = Channel::new_with_members_internal(
        app.clone(),
        vec![remote_name.clone()],
        false,
        Some(conn_id),
        tokio::runtime::Handle::current(),
    )?;
    ```

## Unary-Unary

Send a single request and receive a single response.

=== "Python"

    ```python
    from datetime import timedelta
    from types.example_pb2 import ExampleRequest

    request = ExampleRequest(example_string="world", example_integer=42)
    response = await client.ExampleUnaryUnary(request, timeout=timedelta(seconds=5))
    print("Response:", response.example_string, response.example_integer)
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
    response, err := client.ExampleUnaryUnary(ctx, request)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Println("Response:", response.ExampleString, response.ExampleInteger)
    ```

=== "Java"

    ```java
    import java.time.Duration;
    import com.example_service.ExampleRequest;
    import com.example_service.ExampleResponse;

    ExampleRequest request = ExampleRequest.newBuilder()
        .setExampleString("world")
        .setExampleInteger(42)
        .build();

    ExampleResponse response = client.ExampleUnaryUnary(request).get();
    System.out.println("Response: " + response.getExampleString() + " " + response.getExampleInteger());
    ```

=== "Kotlin"

    ```kotlin
    import com.example_service.ExampleRequest

    val request = ExampleRequest.newBuilder()
        .setExampleString("world")
        .setExampleInteger(42)
        .build()

    val response = client.ExampleUnaryUnary(request)
    println("Response: ${response.exampleString} ${response.exampleInteger}")
    ```

=== ".NET"

    ```csharp
    using ExampleService;

    var request = new ExampleRequest { ExampleString = "world", ExampleInteger = 42 };
    var response = await client.ExampleUnaryUnary(request, timeout: TimeSpan.FromSeconds(5));
    Console.WriteLine($"Response: {response.ExampleString} {response.ExampleInteger}");
    ```

=== "Rust"

    ```rust
    use example_service::{ExampleRequest, ExampleResponse};

    let request = ExampleRequest {
        example_string: "world".to_string(),
        example_integer: 42,
    };

    let response: ExampleResponse = channel
        .unary("example_service.Test", "ExampleUnaryUnary", request, None, None)
        .await?;
    println!("Response: {} {}", response.example_string, response.example_integer);
    ```

## Unary-Stream

Send a single request and iterate over a stream of responses from the server.

=== "Python"

    ```python
    async for response in client.ExampleUnaryStream(request, timeout=timedelta(seconds=5)):
        print("Stream response:", response.example_string, response.example_integer)
    ```

=== "Go"

    ```go
    stream, err := client.ExampleUnaryStream(ctx, request)
    if err != nil {
        log.Fatal(err)
    }
    for {
        resp, err := stream.Recv()
        if err == io.EOF {
            break
        }
        if err != nil {
            log.Fatal(err)
        }
        fmt.Println("Stream response:", resp.ExampleString, resp.ExampleInteger)
    }
    ```

=== "Java"

    ```java
    TestSlimrpc.ClientResponseStream<ExampleResponse> stream =
        client.ExampleUnaryStream(request).get();

    while (stream.hasNext()) {
        ExampleResponse resp = stream.next();
        System.out.println("Stream response: " + resp.getExampleString() + " " + resp.getExampleInteger());
    }
    ```

=== "Kotlin"

    ```kotlin
    client.ExampleUnaryStream(request).collect { resp ->
        println("Stream response: ${resp.exampleString} ${resp.exampleInteger}")
    }
    ```

=== ".NET"

    ```csharp
    await foreach (var response in client.ExampleUnaryStream(request, timeout: TimeSpan.FromSeconds(5)))
    {
        Console.WriteLine($"Stream response: {response.ExampleString} {response.ExampleInteger}");
    }
    ```

=== "Rust"

    ```rust
    let mut stream = channel
        .unary_stream("example_service.Test", "ExampleUnaryStream", request, None, None)
        .await?;

    while let Some(response) = stream.next().await {
        let resp: ExampleResponse = response?;
        println!("Stream response: {} {}", resp.example_string, resp.example_integer);
    }
    ```

## Stream-Unary

Stream a sequence of requests to the server and receive a single response.

=== "Python"

    ```python
    async def stream_requests():
        for i in range(5):
            yield ExampleRequest(example_string=f"req_{i}", example_integer=i)

    response = await client.ExampleStreamUnary(stream_requests(), timeout=timedelta(seconds=5))
    print("Response:", response.example_string, response.example_integer)
    ```

=== "Go"

    ```go
    streamUnary, err := client.ExampleStreamUnary(ctx)
    if err != nil {
        log.Fatal(err)
    }
    for i := 0; i < 5; i++ {
        if err := streamUnary.Send(&pb.ExampleRequest{
            ExampleString:  fmt.Sprintf("req_%d", i),
            ExampleInteger: int64(i),
        }); err != nil {
            log.Fatal(err)
        }
    }
    response, err := streamUnary.CloseAndRecv()
    if err != nil {
        log.Fatal(err)
    }
    fmt.Println("Response:", response.ExampleString, response.ExampleInteger)
    ```

=== "Java"

    ```java
    TestSlimrpc.ClientRequestStream<ExampleRequest, ExampleResponse> requestStream =
        client.ExampleStreamUnary().get();

    for (int i = 0; i < 5; i++) {
        requestStream.send(ExampleRequest.newBuilder()
            .setExampleString("req_" + i)
            .setExampleInteger(i)
            .build());
    }
    ExampleResponse response = requestStream.closeAndReceive().get();
    System.out.println("Response: " + response.getExampleString() + " " + response.getExampleInteger());
    ```

=== "Kotlin"

    ```kotlin
    import kotlinx.coroutines.flow.flow

    val requests = flow {
        for (i in 0 until 5) {
            emit(ExampleRequest.newBuilder()
                .setExampleString("req_$i")
                .setExampleInteger(i.toLong())
                .build())
        }
    }

    val response = client.ExampleStreamUnary(requests)
    println("Response: ${response.exampleString} ${response.exampleInteger}")
    ```

=== ".NET"

    ```csharp
    async IAsyncEnumerable<ExampleRequest> GetRequests()
    {
        for (int i = 0; i < 5; i++)
        {
            yield return new ExampleRequest { ExampleString = $"req_{i}", ExampleInteger = i };
        }
    }

    var response = await client.ExampleStreamUnary(GetRequests(), timeout: TimeSpan.FromSeconds(5));
    Console.WriteLine($"Response: {response.ExampleString} {response.ExampleInteger}");
    ```

=== "Rust"

    ```rust
    let requests: Vec<ExampleRequest> = (0..5)
        .map(|i| ExampleRequest {
            example_string: format!("req_{i}"),
            example_integer: i,
        })
        .collect();

    let response: ExampleResponse = channel
        .stream_unary("example_service.Test", "ExampleStreamUnary", requests, None, None)
        .await?;
    println!("Response: {} {}", response.example_string, response.example_integer);
    ```

## Stream-Stream

Stream requests to the server and receive a stream of responses simultaneously.

=== "Python"

    ```python
    async for response in client.ExampleStreamStream(stream_requests(), timeout=timedelta(seconds=5)):
        print("Stream response:", response.example_string, response.example_integer)
    ```

=== "Go"

    ```go
    import "sync"

    streamBidi, err := client.ExampleStreamStream(ctx)
    if err != nil {
        log.Fatal(err)
    }

    var wg sync.WaitGroup
    wg.Add(1)

    // Send requests in a goroutine
    go func() {
        defer wg.Done()
        for i := 0; i < 5; i++ {
            streamBidi.Send(&pb.ExampleRequest{
                ExampleString:  fmt.Sprintf("req_%d", i),
                ExampleInteger: int64(i),
            })
        }
        streamBidi.CloseSend()
    }()

    // Receive responses
    for {
        resp, err := streamBidi.Recv()
        if err == io.EOF {
            break
        }
        if err != nil {
            log.Fatal(err)
        }
        fmt.Println("Stream response:", resp.ExampleString, resp.ExampleInteger)
    }
    wg.Wait()
    ```

=== "Java"

    ```java
    TestSlimrpc.ClientBidiStream<ExampleRequest> bidiStream =
        client.ExampleStreamStream().get();

    // Send requests
    for (int i = 0; i < 5; i++) {
        bidiStream.send(ExampleRequest.newBuilder()
            .setExampleString("req_" + i)
            .setExampleInteger(i)
            .build());
    }
    bidiStream.closeSend();

    // Receive responses
    ExampleResponse resp;
    while ((resp = bidiStream.recv()) != null) {
        System.out.println("Stream response: " + resp.getExampleString() + " " + resp.getExampleInteger());
    }
    ```

=== "Kotlin"

    ```kotlin
    client.ExampleStreamStream(requests).collect { resp ->
        println("Stream response: ${resp.exampleString} ${resp.exampleInteger}")
    }
    ```

=== ".NET"

    ```csharp
    await foreach (var response in client.ExampleStreamStream(GetRequests(), timeout: TimeSpan.FromSeconds(5)))
    {
        Console.WriteLine($"Stream response: {response.ExampleString} {response.ExampleInteger}");
    }
    ```

=== "Rust"

    ```rust
    let mut stream = channel
        .stream_stream("example_service.Test", "ExampleStreamStream", requests, None, None)
        .await?;

    while let Some(response) = stream.next().await {
        let resp: ExampleResponse = response?;
        println!("Stream response: {} {}", resp.example_string, resp.example_integer);
    }
    ```

## Close the Channel

When finished, close the channel to release the underlying SLIM session.

=== "Python"

    ```python
    await channel.close_async(timeout=None)
    ```

=== "Go"

    ```go
    channel.Close()
    ```

=== "Java"

    ```java
    channel.close();
    ```

=== "Kotlin"

    ```kotlin
    channel.close()
    ```

=== ".NET"

    ```csharp
    channel.Dispose();
    ```

=== "Rust"

    ```rust
    channel.close().await?;
    ```

## Runnable Examples

Complete client examples for each language:

- [Python client example](https://github.com/agntcy/slim-bindings/blob/main/python/examples/slimrpc/simple/client.py)
- [Go client example](https://github.com/agntcy/slim-bindings/blob/main/go/examples/slimrpc/simple/cmd/client/client.go)
- [Java/Kotlin client example](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples/slimrpc/simple)
- [.NET client example](https://github.com/agntcy/slim-bindings/tree/main/dotnet/Slim.Examples.SlimRpc)

## Next Steps

- [Multicast SLIMRPC](./tutorial-multicast.md) — Fan out a single call to multiple servers simultaneously
- [SLIMRPC](../../slimrpc/index.md) — Naming scheme, under-the-hood details, and multicast RPC
