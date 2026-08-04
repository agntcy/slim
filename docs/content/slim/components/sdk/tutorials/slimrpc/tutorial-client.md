# Tutorial: Using a SLIMRPC Server

This tutorial shows how to connect to a running SLIMRPC server, create a channel, and call each of the four RPC patterns: unary-unary, unary-stream, stream-unary, and stream-stream.

## Prerequisites

- Completed [Connecting to SLIM](../tutorial-connect.md) and [Creating an App](../tutorial-app.md) — you need the `app` and `conn_id` objects
- A running SLIMRPC server — see [Serving a SLIMRPC Server](./tutorial-serve.md)
- Generated client stubs for your service — see Step 2 of the serving tutorial

## Create a SLIMRPC Channel

A SLIMRPC channel wraps the SLIM session layer. Pass it the remote server's SLIM name and the connection ID from `connect_async`.

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
        slim_rpc "github.com/agntcy/slim-bindings-go/v2/slim_rpc"
        pb "example/types"
    )

    // app, connId, and remoteName come from the prerequisite tutorials
    channel := slim_rpc.ChannelNewWithConnection(app, remoteName, &connId)
    client := pb.NewTestClient(channel)
    ```

=== "Java"

    ```java
    import io.agntcy.slim.bindings.slimrpc.Channel;
    import com.example_service.TestSlimrpc;

    // app, connId, and remoteName come from the prerequisite tutorials
    Channel channel = Channel.newWithConnection(app, remoteName, connId);
    TestSlimrpc.TestClientImpl client = new TestSlimrpc.TestClientImpl(channel);
    ```

=== "Kotlin"

    ```kotlin
    import io.agntcy.slim.bindings.slimrpc.Channel
    import com.example_service.TestSlimrpc

    // app, connId, and remoteName come from the prerequisite tutorials
    val channel = Channel.newWithConnection(app, remoteName, connId)
    val client = TestSlimrpc.TestClientImpl(channel)
    ```

=== "Node.js"

    ```typescript
    import slimBindings from '@agntcy/slim-bindings';
    import { TestClient } from './types/example_slimrpc.js';

    // app, connId, and remoteName come from the prerequisite tutorials
    const channel = slimBindings.Channel.newWithConnection(app, remoteName, connId);
    const client = new TestClient(channel);
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim.SlimRpc;
    using ExampleService;

    // app, connId, and remoteName come from the prerequisite tutorials
    var channel = SlimRpcChannelFactory.CreateChannel(app, remoteName, connId);
    var client = new TestClient(channel);
    ```

## Unary-Unary

Send a single request and receive a single response.

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

    ExampleResponse response = client.ExampleUnaryUnary(request, Duration.ofSeconds(5), null);
    System.out.println("Response: " + response.getExampleString() + " " + response.getExampleInteger());
    ```

=== "Kotlin"

    ```kotlin
    import com.example_service.ExampleRequest
    import java.time.Duration

    val request = ExampleRequest.newBuilder()
        .setExampleString("world")
        .setExampleInteger(42)
        .build()

    val response = client.ExampleUnaryUnary(request, Duration.ofSeconds(5), null)
    println("Response: ${response.exampleString} ${response.exampleInteger}")
    ```

=== "Node.js"

    ```typescript
    import { create } from '@bufbuild/protobuf';
    import { ExampleRequestSchema } from './types/example_pb.js';

    const request = create(ExampleRequestSchema, { exampleString: 'world', exampleInteger: 42n });
    const response = await client.ExampleUnaryUnary(request, 5000);
    console.log('Response:', response.exampleString, response.exampleInteger);
    ```

=== ".NET"

    ```csharp
    using ExampleService;

    var request = new ExampleRequest { ExampleString = "world", ExampleInteger = 42 };
    var response = await client.ExampleUnaryUnaryAsync(request, timeout: TimeSpan.FromSeconds(5));
    Console.WriteLine($"Response: {response.ExampleString} {response.ExampleInteger}");
    ```

## Unary-Stream

Send a single request and iterate over a stream of responses from the server.

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
    import io.agntcy.slim.bindings.slimrpc.ClientResponseStream;
    import io.agntcy.slim.bindings.slimrpc.ResponseStreamReader;

    ResponseStreamReader streamReader = client.ExampleUnaryStream(request, Duration.ofSeconds(5), null);
    ClientResponseStream<ExampleResponse> stream = ClientResponseStream.create(streamReader,
        bytes -> {
            try { return ExampleResponse.parseFrom(bytes); }
            catch (Exception e) { throw new RuntimeException(e); }
        });
    while (true) {
        ExampleResponse resp = stream.recv();
        if (resp == null) break;
        System.out.println("Stream response: " + resp.getExampleString() + " " + resp.getExampleInteger());
    }
    ```

=== "Kotlin"

    ```kotlin
    import io.agntcy.slim.bindings.slimrpc.ClientResponseStream
    import java.time.Duration

    val streamReader = client.ExampleUnaryStream(request, Duration.ofSeconds(5), null)
    val stream = ClientResponseStream.create(streamReader) { bytes -> ExampleResponse.parseFrom(bytes) }
    while (true) {
        val resp = stream.recv() ?: break
        println("Stream response: ${resp.exampleString} ${resp.exampleInteger}")
    }
    ```

=== "Node.js"

    ```typescript
    for await (const resp of client.ExampleUnaryStream(request, 5000)) {
        console.log('Stream response:', resp.exampleString, resp.exampleInteger);
    }
    ```

=== ".NET"

    ```csharp
    await foreach (var response in client.ExampleUnaryStreamAsync(request, timeout: TimeSpan.FromSeconds(5)))
    {
        Console.WriteLine($"Stream response: {response.ExampleString} {response.ExampleInteger}");
    }
    ```

## Stream-Unary

Stream a sequence of requests to the server and receive a single response.

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
    import io.agntcy.slim.bindings.slimrpc.ClientRequestStream;

    ClientRequestStream<ExampleRequest, ExampleResponse> stream =
        client.ExampleStreamUnary(Duration.ofSeconds(5), null);

    for (int i = 0; i < 5; i++) {
        stream.send(ExampleRequest.newBuilder()
            .setExampleString("req_" + i)
            .setExampleInteger(i)
            .build());
    }
    ExampleResponse response = stream.finalizeStream();
    System.out.println("Response: " + response.getExampleString() + " " + response.getExampleInteger());
    ```

=== "Kotlin"

    ```kotlin
    import io.agntcy.slim.bindings.slimrpc.ClientRequestStream
    import java.time.Duration

    val stream: ClientRequestStream<ExampleRequest, ExampleResponse> =
        client.ExampleStreamUnary(Duration.ofSeconds(5), null)

    for (i in 0 until 5) {
        stream.send(ExampleRequest.newBuilder()
            .setExampleString("req_$i")
            .setExampleInteger(i.toLong())
            .build())
    }
    val response = stream.finalizeStream()
    println("Response: ${response.exampleString} ${response.exampleInteger}")
    ```

=== "Node.js"

    ```typescript
    async function* streamRequests() {
        for (let i = 0; i < 5; i++) {
            yield create(ExampleRequestSchema, { exampleString: `req_${i}`, exampleInteger: BigInt(i) });
        }
    }

    const response = await client.ExampleStreamUnary(streamRequests(), 5000);
    console.log('Response:', response.exampleString, response.exampleInteger);
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

    var response = await client.ExampleStreamUnaryAsync(GetRequests(), timeout: TimeSpan.FromSeconds(5));
    Console.WriteLine($"Response: {response.ExampleString} {response.ExampleInteger}");
    ```

## Stream-Stream

Stream requests to the server and receive a stream of responses simultaneously.

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
    import io.agntcy.slim.bindings.slimrpc.ClientBidiStream;
    import io.agntcy.slim.bindings.slimrpc.StreamMessage;

    ClientBidiStream<ExampleRequest> stream = client.ExampleStreamStream(Duration.ofSeconds(5), null);

    // Send requests in a separate thread
    new Thread(() -> {
        for (int i = 0; i < 5; i++) {
            stream.send(ExampleRequest.newBuilder()
                .setExampleString("req_" + i)
                .setExampleInteger(i)
                .build());
        }
        stream.closeSend();
    }).start();

    // Receive responses
    while (true) {
        StreamMessage msg = stream.recv();
        if (msg instanceof StreamMessage.End) break;
        if (msg instanceof StreamMessage.Error err) throw new RuntimeException(err.v1().toString());
        if (msg instanceof StreamMessage.Data data) {
            ExampleResponse resp = ExampleResponse.parseFrom(data.v1());
            System.out.println("Stream response: " + resp.getExampleString() + " " + resp.getExampleInteger());
        }
    }
    ```

=== "Kotlin"

    ```kotlin
    import io.agntcy.slim.bindings.slimrpc.ClientBidiStream
    import io.agntcy.slim.bindings.slimrpc.StreamMessage
    import java.time.Duration
    import kotlinx.coroutines.coroutineScope
    import kotlinx.coroutines.launch

    coroutineScope {
        val stream: ClientBidiStream<ExampleRequest> = client.ExampleStreamStream(Duration.ofSeconds(5), null)

        // Send requests in a launched coroutine
        launch {
            for (i in 0 until 5) {
                stream.send(ExampleRequest.newBuilder()
                    .setExampleString("req_$i")
                    .setExampleInteger(i.toLong())
                    .build())
            }
            stream.closeSend()
        }

        // Receive responses
        while (true) {
            when (val msg = stream.recv()) {
                is StreamMessage.End -> break
                is StreamMessage.Error -> throw RuntimeException(msg.v1.toString())
                is StreamMessage.Data -> {
                    val resp = ExampleResponse.parseFrom(msg.v1)
                    println("Stream response: ${resp.exampleString} ${resp.exampleInteger}")
                }
            }
        }
    }
    ```

=== "Node.js"

    ```typescript
    for await (const resp of client.ExampleStreamStream(streamRequests(), 5000)) {
        console.log('Stream response:', resp.exampleString, resp.exampleInteger);
    }
    ```

=== ".NET"

    ```csharp
    await foreach (var response in client.ExampleStreamStreamAsync(GetRequests(), timeout: TimeSpan.FromSeconds(5)))
    {
        Console.WriteLine($"Stream response: {response.ExampleString} {response.ExampleInteger}");
    }
    ```

## Close the Channel

When finished, close the channel to release the underlying SLIM session.

=== "Rust"

    ```rust
    channel.close().await?;
    ```

=== "Python"

    ```python
    await channel.close_async(timeout=None)
    ```

=== "Go"

    ```go
    channel.CloseBlocking(nil)
    ```

=== "Java"

    ```java
    channel.close();
    ```

=== "Kotlin"

    ```kotlin
    channel.close()
    ```

=== "Node.js"

    ```typescript
    await channel.closeAsync(undefined);
    ```

=== ".NET"

    ```csharp
    channel.Dispose();
    ```

## Runnable Examples

Complete client examples for each language:

- [Python client example](https://github.com/agntcy/slim-bindings/blob/main/python/examples/slimrpc/simple/client.py)
- [Go client example](https://github.com/agntcy/slim-bindings/blob/main/go/examples/slimrpc/simple/cmd/client/client.go)
- [Java/Kotlin client example](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples/slimrpc/simple)
- [Node.js client example](https://github.com/agntcy/slim-bindings/blob/main/node/examples/slimrpc/simple/client.ts)
- [.NET client example](https://github.com/agntcy/slim-bindings/tree/main/dotnet/Slim.Examples.SlimRpc)

## Next Steps

- [Multicast SLIMRPC](./tutorial-multicast.md) — Fan out a single call to multiple servers simultaneously
- [SLIMRPC](../../slimrpc/index.md) — Naming scheme, under-the-hood details, and multicast RPC
