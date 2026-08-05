# Tutorial: Serving a SLIMRPC Server

This tutorial shows how to define a protobuf service, generate SLIMRPC bindings, implement the server logic, and start accepting calls. SLIMRPC runs over the SLIM session layer, so there is no separate networking configuration — discovery and transport are handled automatically.

## Prerequisites

- Completed [Connecting to SLIM](../tutorial-connect.md) and [Creating an App](../tutorial-app.md) — you need the `app` and `conn_id` objects
- SLIMRPC compiler installed — see [Compiler](../../slimrpc/compiler.md)
- [`buf`](https://buf.build/docs/installation) installed (or `protoc`)

## Step 1: Define a Proto

Create a `.proto` file that defines your service. SLIMRPC supports all four streaming patterns:

```proto
syntax = "proto3";

package example_service;

service Test {
  rpc ExampleUnaryUnary(ExampleRequest)          returns (ExampleResponse);
  rpc ExampleUnaryStream(ExampleRequest)         returns (stream ExampleResponse);
  rpc ExampleStreamUnary(stream ExampleRequest)  returns (ExampleResponse);
  rpc ExampleStreamStream(stream ExampleRequest) returns (stream ExampleResponse);
}

message ExampleRequest {
  string example_string = 1;
  int64  example_integer = 2;
}

message ExampleResponse {
  string example_string = 1;
  int64  example_integer = 2;
}
```

The server subscribes to a single SLIM base name. The service (`{package}.{Service}`) and method name are carried as metadata on each message, so a single subscription handles all methods. See [SLIMRPC](../../slimrpc/index.md) for details.

## Step 2: Generate Code Using the Compiler

Create a `buf.gen.yaml` to generate SLIMRPC stubs alongside the standard protobuf types, then run `buf generate`.

=== "Rust"

    In Rust, SLIMRPC handlers are registered as closures directly on the server — no SLIMRPC compiler or generated stub files are needed.

    You still generate the protobuf message types from your `.proto` file. Add `prost` to your `Cargo.toml` and a `build.rs`:

    ```toml
    # Cargo.toml
    [dependencies]
    prost = "0.13"
    agntcy-slim = "2.0.0"
    tokio = { version = "1", features = ["full"] }

    [build-dependencies]
    prost-build = "0.13"
    ```

    ```rust
    // build.rs
    fn main() {
        prost_build::compile_protos(&["proto/example.proto"], &["proto/"]).unwrap();
    }
    ```

    This generates Rust structs for `ExampleRequest` and `ExampleResponse`. Handler registration is shown in Step 4.

=== "Python"

    ```yaml
    # buf.gen.yaml
    version: v2
    managed:
      enabled: true
    inputs:
      - proto_file: example.proto
    plugins:
      - local: protoc-gen-slimrpc-python
        out: types
      - remote: buf.build/protocolbuffers/python:v29.3
        out: types
      - remote: buf.build/protocolbuffers/pyi:v31.1
        out: types
    ```

    This generates `types/example_pb2.py` (protobuf types) and `types/example_pb2_slimrpc.py` (SLIMRPC stubs).

=== "Go"

    ```yaml
    # buf.gen.yaml
    version: v2
    managed:
      enabled: true
    plugins:
      - remote: buf.build/protocolbuffers/go
        out: types
        opt:
          - paths=source_relative
      - local: protoc-gen-slimrpc-go
        out: types
        opt:
          - paths=source_relative
    ```

    This generates `types/example.pb.go` (protobuf types) and `types/example_slimrpc.pb.go` (SLIMRPC stubs).

=== "Java"

    ```yaml
    # buf.gen.yaml
    version: v2
    managed:
      enabled: true
    plugins:
      - remote: buf.build/protocolbuffers/java
        out: src/main/java
      - local: protoc-gen-slimrpc-java
        out: src/main/java
    ```

    This generates the standard protobuf Java classes and a `TestSlimrpc.java` file containing the client, server, and registration function.

=== "Kotlin"

    ```yaml
    # buf.gen.yaml
    version: v2
    managed:
      enabled: true
    plugins:
      - remote: buf.build/protocolbuffers/java
        out: src/main/java
      - remote: buf.build/protocolbuffers/kotlin
        out: src/main/kotlin
      - local: protoc-gen-slimrpc-kotlin
        out: slimrpc
    ```

    This generates the Java protobuf classes, Kotlin protobuf extensions, and a `TestSlimrpc.kt` file with coroutine-based client, server, and registration function.

=== "Node.js"

    ```yaml
    # buf.gen.yaml
    version: v2
    managed:
      enabled: true
    plugins:
      - local: protoc-gen-slimrpc-node
        out: types
      - remote: buf.build/bufbuild/es:v2.12.1
        out: types
        opt:
          - target=ts
          - import_extension=js
    ```

    This generates `types/example_pb.ts` (protobuf-es message types) and `types/example_slimrpc.ts` containing `TestServicer`, `registerTestServicer`, `TestClient`, and `TestGroupClient`.

=== ".NET"

    ```yaml
    # buf.gen.yaml
    version: v2
    managed:
      enabled: true
    plugins:
      - remote: buf.build/protocolbuffers/csharp
        out: Generated
        opt: base_namespace=ExampleService
      - local: protoc-gen-slimrpc-csharp
        out: Generated
        opt: base_namespace=ExampleService,types_namespace=ExampleService
    ```

    This generates the C# protobuf classes and `example_slimrpc.cs` containing `TestClient`, `ITestServer`, `UnimplementedTestServer`, and `TestServerRegistration`.

Run code generation:

```bash
buf generate
```

## Step 3: Implement the Servicer

Implement each RPC method defined in your proto. Extend the generated base class or implement the interface:

=== "Rust"

    In Rust, service logic is written as async closures passed to the server at registration time (Step 4). Each handler receives a typed request and returns a typed response — no base class to extend.

    ```rust
    // include! the prost-generated types at the top of your file:
    // mod example_service { include!(concat!(env!("OUT_DIR"), "/example_service.rs")); }
    use example_service::{ExampleRequest, ExampleResponse};

    // Handlers are closures — shown registered in Step 4 below.
    // ExampleUnaryUnary:
    //   |req: ExampleRequest, _ctx| async move { Ok(ExampleResponse { ... }) }
    //
    // ExampleUnaryStream:
    //   |req: ExampleRequest, _ctx| async move { Ok(vec![ExampleResponse { ... }]) }
    //
    // ExampleStreamUnary:
    //   |reqs: Vec<ExampleRequest>, _ctx| async move { Ok(ExampleResponse { ... }) }
    //
    // ExampleStreamStream:
    //   |reqs: Vec<ExampleRequest>, _ctx| async move { Ok(reqs.into_iter().map(|r| ...).collect()) }
    ```

=== "Python"

    ```python
    import asyncio
    from typing import AsyncIterable
    from types.example_pb2 import ExampleRequest, ExampleResponse
    from types.example_pb2_slimrpc import TestServicer, add_TestServicer_to_server

    class TestService(TestServicer):
        async def ExampleUnaryUnary(
            self, request: ExampleRequest, context
        ) -> ExampleResponse:
            return ExampleResponse(
                example_string=f"hello {request.example_string}",
                example_integer=request.example_integer + 1,
            )

        async def ExampleUnaryStream(
            self, request: ExampleRequest, context
        ) -> AsyncIterable[ExampleResponse]:
            for i in range(5):
                yield ExampleResponse(
                    example_string=f"hello {request.example_string} {i}",
                    example_integer=request.example_integer + i,
                )

        async def ExampleStreamUnary(
            self, request_iterator, context
        ) -> ExampleResponse:
            count = 0
            async for req in request_iterator:
                count += 1
            return ExampleResponse(
                example_string=f"received {count} requests",
                example_integer=count,
            )

        async def ExampleStreamStream(
            self, request_iterator, context
        ) -> AsyncIterable[ExampleResponse]:
            async for req in request_iterator:
                yield ExampleResponse(
                    example_string=f"echo: {req.example_string}",
                    example_integer=req.example_integer,
                )
    ```

=== "Go"

    ```go
    import (
        "context"
        "fmt"
        "io"

        pb "example/types"
        slim_rpc "github.com/agntcy/slim-bindings-go/v2/slim_rpc"
    )

    type TestServiceImpl struct {
        pb.UnimplementedTestServer
    }

    func (s *TestServiceImpl) ExampleUnaryUnary(
        ctx context.Context, req *pb.ExampleRequest,
    ) (*pb.ExampleResponse, error) {
        return &pb.ExampleResponse{
            ExampleString:  "hello " + req.ExampleString,
            ExampleInteger: req.ExampleInteger + 1,
        }, nil
    }

    func (s *TestServiceImpl) ExampleUnaryStream(
        ctx context.Context, req *pb.ExampleRequest,
        stream slim_rpc.ServerStream[*pb.ExampleResponse],
    ) error {
        for i := 0; i < 5; i++ {
            if err := stream.Send(&pb.ExampleResponse{
                ExampleString:  fmt.Sprintf("hello %s %d", req.ExampleString, i),
                ExampleInteger: req.ExampleInteger + int64(i),
            }); err != nil {
                return err
            }
        }
        return nil
    }

    func (s *TestServiceImpl) ExampleStreamUnary(
        ctx context.Context, stream slim_rpc.ResponseStream[*pb.ExampleRequest],
    ) (*pb.ExampleResponse, error) {
        count := int64(0)
        for {
            _, err := stream.Recv()
            if err == io.EOF {
                break
            }
            if err != nil {
                return nil, err
            }
            count++
        }
        return &pb.ExampleResponse{
            ExampleString:  fmt.Sprintf("received %d requests", count),
            ExampleInteger: count,
        }, nil
    }

    func (s *TestServiceImpl) ExampleStreamStream(
        ctx context.Context,
        stream slim_rpc.ServerBidiStream[*pb.ExampleRequest, *pb.ExampleResponse],
    ) error {
        for {
            req, err := stream.Recv()
            if err == io.EOF {
                break
            }
            if err != nil {
                return err
            }
            if err := stream.Send(&pb.ExampleResponse{
                ExampleString:  "echo: " + req.ExampleString,
                ExampleInteger: req.ExampleInteger,
            }); err != nil {
                return err
            }
        }
        return nil
    }
    ```

=== "Java"

    ```java
    import java.util.concurrent.CompletableFuture;
    import com.example_service.TestSlimrpc;
    import com.example_service.ExampleRequest;
    import com.example_service.ExampleResponse;
    import io.agntcy.slim.bindings.slimrpc.*;

    class TestServerImpl implements TestSlimrpc.TestServer {
        @Override
        public CompletableFuture<ExampleResponse> ExampleUnaryUnary(
                ExampleRequest request, Context context) {
            return CompletableFuture.completedFuture(
                ExampleResponse.newBuilder()
                    .setExampleString("hello " + request.getExampleString())
                    .setExampleInteger(request.getExampleInteger() + 1)
                    .build()
            );
        }

        @Override
        public CompletableFuture<Void> ExampleUnaryStream(
                ExampleRequest request, Context context, ResponseSink sink) {
            ServerRequestStream<ExampleResponse> sender = ServerRequestStream.create(
                sink, ExampleResponse::toByteArray);
            for (int i = 0; i < 5; i++) {
                try {
                    sender.send(ExampleResponse.newBuilder()
                        .setExampleString("hello " + request.getExampleString() + " " + i)
                        .setExampleInteger(request.getExampleInteger() + i)
                        .build());
                } catch (Exception e) {
                    return CompletableFuture.failedFuture(e);
                }
            }
            return CompletableFuture.completedFuture(null);
        }

        @Override
        public CompletableFuture<ExampleResponse> ExampleStreamUnary(
                RequestStream stream, Context context) {
            ServerResponseStream<ExampleRequest> reader = ServerResponseStream.create(stream,
                bytes -> {
                    try { return ExampleRequest.parseFrom(bytes); }
                    catch (Exception e) { throw new RuntimeException(e); }
                });
            long count = 0;
            while (true) {
                ExampleRequest req;
                try { req = reader.recv(); } catch (Exception e) { return CompletableFuture.failedFuture(e); }
                if (req == null) break;
                count++;
            }
            return CompletableFuture.completedFuture(
                ExampleResponse.newBuilder()
                    .setExampleString("received " + count + " requests")
                    .setExampleInteger(count)
                    .build());
        }

        @Override
        public CompletableFuture<Void> ExampleStreamStream(
                RequestStream stream, Context context, ResponseSink sink) {
            ServerBidiStream<ExampleRequest, ExampleResponse> bidi = ServerBidiStream.create(stream, sink,
                bytes -> {
                    try { return ExampleRequest.parseFrom(bytes); }
                    catch (Exception e) { throw new RuntimeException(e); }
                },
                ExampleResponse::toByteArray);
            while (true) {
                ExampleRequest req;
                try { req = bidi.recv(); } catch (Exception e) { return CompletableFuture.failedFuture(e); }
                if (req == null) break;
                try {
                    bidi.send(ExampleResponse.newBuilder()
                        .setExampleString("echo: " + req.getExampleString())
                        .setExampleInteger(req.getExampleInteger())
                        .build());
                } catch (Exception e) { return CompletableFuture.failedFuture(e); }
            }
            return CompletableFuture.completedFuture(null);
        }
    }
    ```

=== "Kotlin"

    ```kotlin
    import com.example_service.TestSlimrpc
    import com.example_service.ExampleRequest
    import com.example_service.ExampleResponse
    import io.agntcy.slim.bindings.slimrpc.*

    class TestServiceImpl : TestSlimrpc.UnimplementedTestServer() {
        override suspend fun ExampleUnaryUnary(
            request: ExampleRequest, context: Context
        ): ExampleResponse = ExampleResponse.newBuilder()
            .setExampleString("hello ${request.exampleString}")
            .setExampleInteger(request.exampleInteger + 1)
            .build()

        override suspend fun ExampleUnaryStream(
            request: ExampleRequest, context: Context, sink: ResponseSink
        ) {
            val sender = ServerRequestStream.create(sink) { resp: ExampleResponse -> resp.toByteArray() }
            for (i in 0 until 5) {
                sender.send(ExampleResponse.newBuilder()
                    .setExampleString("hello ${request.exampleString} $i")
                    .setExampleInteger(request.exampleInteger + i)
                    .build())
            }
        }

        override suspend fun ExampleStreamUnary(
            stream: RequestStream, context: Context
        ): ExampleResponse {
            val reader = ServerResponseStream.create(stream) { ExampleRequest.parseFrom(it) }
            var count = 0L
            while (reader.recv() != null) { count++ }
            return ExampleResponse.newBuilder()
                .setExampleString("received $count requests")
                .setExampleInteger(count)
                .build()
        }

        override suspend fun ExampleStreamStream(
            stream: RequestStream, context: Context, sink: ResponseSink
        ) {
            val bidi = ServerBidiStream.create(
                stream, sink,
                { ExampleRequest.parseFrom(it) },
                { resp: ExampleResponse -> resp.toByteArray() }
            )
            while (true) {
                val req = bidi.recv() ?: break
                bidi.send(ExampleResponse.newBuilder()
                    .setExampleString("echo: ${req.exampleString}")
                    .setExampleInteger(req.exampleInteger)
                    .build())
            }
        }
    }
    ```

=== "Node.js"

    ```typescript
    import type { ContextLike } from '@agntcy/slim-bindings';
    import { create } from '@bufbuild/protobuf';
    import { ExampleResponseSchema, type ExampleRequest, type ExampleResponse } from './types/example_pb.js';
    import { registerTestServicer, type TestServicer } from './types/example_slimrpc.js';

    class TestService implements TestServicer {
        async ExampleUnaryUnary(request: ExampleRequest, context: ContextLike): Promise<ExampleResponse> {
            return create(ExampleResponseSchema, {
                exampleString: `hello ${request.exampleString}`,
                exampleInteger: request.exampleInteger + 1n,
            });
        }

        async *ExampleUnaryStream(request: ExampleRequest, context: ContextLike): AsyncIterable<ExampleResponse> {
            for (let i = 0; i < 5; i++) {
                yield create(ExampleResponseSchema, {
                    exampleString: `hello ${request.exampleString} ${i}`,
                    exampleInteger: request.exampleInteger + BigInt(i),
                });
            }
        }

        async ExampleStreamUnary(requests: AsyncIterable<ExampleRequest>, context: ContextLike): Promise<ExampleResponse> {
            let count = 0n;
            for await (const _ of requests) { count++; }
            return create(ExampleResponseSchema, {
                exampleString: `received ${count} requests`,
                exampleInteger: count,
            });
        }

        async *ExampleStreamStream(requests: AsyncIterable<ExampleRequest>, context: ContextLike): AsyncIterable<ExampleResponse> {
            for await (const req of requests) {
                yield create(ExampleResponseSchema, {
                    exampleString: `echo: ${req.exampleString}`,
                    exampleInteger: req.exampleInteger,
                });
            }
        }
    }
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim.SlimRpc;
    using ExampleService;

    class TestServerImpl : ITestServer
    {
        public async Task<ExampleResponse> ExampleUnaryUnary(
            ExampleRequest request, SlimRpcContext context)
        {
            return new ExampleResponse
            {
                ExampleString = $"hello {request.ExampleString}",
                ExampleInteger = request.ExampleInteger + 1
            };
        }

        public async IAsyncEnumerable<ExampleResponse> ExampleUnaryStream(
            ExampleRequest request, SlimRpcContext context)
        {
            for (int i = 0; i < 5; i++)
            {
                yield return new ExampleResponse
                {
                    ExampleString = $"hello {request.ExampleString} {i}",
                    ExampleInteger = request.ExampleInteger + i
                };
            }
        }

        public async Task<ExampleResponse> ExampleStreamUnary(
            IAsyncEnumerable<ExampleRequest> requestStream, SlimRpcContext context)
        {
            long count = 0;
            await foreach (var _ in requestStream)
                count++;
            return new ExampleResponse
            {
                ExampleString = $"received {count} requests",
                ExampleInteger = count
            };
        }

        public async IAsyncEnumerable<ExampleResponse> ExampleStreamStream(
            IAsyncEnumerable<ExampleRequest> requestStream, SlimRpcContext context)
        {
            await foreach (var req in requestStream)
            {
                yield return new ExampleResponse
                {
                    ExampleString = $"echo: {req.ExampleString}",
                    ExampleInteger = req.ExampleInteger
                };
            }
        }
    }
    ```

## Step 4: Create the Server and Serve

Create a SLIMRPC server, register your implementation, and start serving. The server blocks (or runs asynchronously) until stopped.

=== "Rust"

    ```rust
    use slim_rpc::Server;
    use example_service::{ExampleRequest, ExampleResponse};

    // app, conn_id, and rx (notification receiver) come from the prerequisite tutorials
    let server = Server::new_with_connection_and_runtime(
        app,
        local_name,
        Some(conn_id),
        rx,
        None,
    );

    // Register each RPC method as a typed async closure
    server.register_unary_unary_internal(
        "example_service.Test",
        "ExampleUnaryUnary",
        |req: ExampleRequest, _ctx| async move {
            Ok(ExampleResponse {
                example_string: format!("hello {}", req.example_string),
                example_integer: req.example_integer + 1,
            })
        },
    );

    server.register_unary_stream_internal(
        "example_service.Test",
        "ExampleUnaryStream",
        |req: ExampleRequest, _ctx| async move {
            let responses = (0..5)
                .map(|i| ExampleResponse {
                    example_string: format!("hello {} {i}", req.example_string),
                    example_integer: req.example_integer + i,
                })
                .collect::<Vec<_>>();
            Ok(responses)
        },
    );

    server.register_stream_unary_internal(
        "example_service.Test",
        "ExampleStreamUnary",
        |reqs: Vec<ExampleRequest>, _ctx| async move {
            let count = reqs.len() as i64;
            Ok(ExampleResponse {
                example_string: format!("received {count} requests"),
                example_integer: count,
            })
        },
    );

    server.register_stream_stream_internal(
        "example_service.Test",
        "ExampleStreamStream",
        |reqs: Vec<ExampleRequest>, _ctx| async move {
            let responses = reqs
                .into_iter()
                .map(|req| ExampleResponse {
                    example_string: format!("echo: {}", req.example_string),
                    example_integer: req.example_integer,
                })
                .collect::<Vec<_>>();
            Ok(responses)
        },
    );

    println!("Serving...");
    server.serve().await?;
    ```

=== "Python"

    ```python
    import slim_bindings
    from types.example_pb2_slimrpc import add_TestServicer_to_server

    # app and conn_id come from the prerequisite tutorials
    rpc_server = slim_bindings.Server.new_with_connection(app, local_name, conn_id)
    add_TestServicer_to_server(TestService(), rpc_server)

    print("Serving...")
    await rpc_server.serve_async()
    ```

=== "Go"

    ```go
    import slim_rpc "github.com/agntcy/slim-bindings-go/v2/slim_rpc"

    // app and connId come from the prerequisite tutorials
    server := slim_rpc.ServerNewWithConnection(app, localName, &connId)
    pb.RegisterTestServer(server, &TestServiceImpl{})

    fmt.Println("Serving...")
    server.ServeBlocking()
    ```

=== "Java"

    ```java
    import io.agntcy.slim.bindings.slimrpc.Server;

    // app and connId come from the prerequisite tutorials
    Server rpcServer = Server.newWithConnection(app, localName, connId);
    TestSlimrpc.registerTestServer(rpcServer, new TestServerImpl());

    System.out.println("Serving...");
    rpcServer.serveBlocking();
    ```

=== "Kotlin"

    ```kotlin
    import io.agntcy.slim.bindings.slimrpc.Server

    // app and connId come from the prerequisite tutorials
    val rpcServer = Server.newWithConnection(app, localName, connId)
    TestSlimrpc.registerTestServer(rpcServer, TestServiceImpl())

    println("Serving...")
    rpcServer.serveBlocking()
    ```

=== "Node.js"

    ```typescript
    import slimBindings from '@agntcy/slim-bindings';

    // app, localName, and connId come from the prerequisite tutorials
    const rpcServer = slimBindings.Server.newWithConnection(app, localName, connId);
    registerTestServicer(rpcServer, new TestService());

    console.log('Serving...');
    await rpcServer.serveAsync();
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim.SlimRpc;
    using ExampleService;

    // app and connId come from the prerequisite tutorials
    var slimServer = SlimRpcServerFactory.CreateServer(app, localName, connId);
    TestServerRegistration.RegisterTestServer(slimServer, new TestServerImpl());

    Console.WriteLine("Serving...");
    await slimServer.ServeAsync();
    ```

## Runnable Examples

Complete server examples for each language:

- [Python server example](https://github.com/agntcy/slim-bindings/blob/main/python/examples/slimrpc/simple/server.py)
- [Go server example](https://github.com/agntcy/slim-bindings/blob/main/go/examples/slimrpc/simple/cmd/server/server.go)
- [Java/Kotlin server example](https://github.com/agntcy/slim-bindings/tree/main/kotlin/examples/slimrpc/simple)
- [Node.js server example](https://github.com/agntcy/slim-bindings/blob/main/node/examples/slimrpc/simple/server.ts)
- [.NET server example](https://github.com/agntcy/slim-bindings/tree/main/dotnet/Slim.Examples.SlimRpc)

## Next Steps

- [Using a SLIMRPC Server](./tutorial-client.md) — Create a channel and call your server from a client
- [SLIMRPC](../../slimrpc/index.md) — Naming scheme, under-the-hood details, and multicast RPC
