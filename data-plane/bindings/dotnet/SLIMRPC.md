# slimrpc (SLIM Remote Procedure Call)

slimrpc, or SLIM Remote Procedure Call, is a mechanism designed to enable Protocol
Buffers (protobuf) RPC over SLIM (Secure Low-Latency Interactive Messaging).
This is analogous to gRPC, which leverages HTTP/2 as its underlying transport
layer for protobuf RPC.

A key advantage of slimrpc lies in its ability to seamlessly integrate SLIM as the
transport protocol for inter-application message exchange. This significantly
simplifies development: a protobuf file can be compiled to generate code that
utilizes SLIM for communication. Application developers can then interact with
the generated code much like they would with standard gRPC, while benefiting
from the inherent security features and efficiency provided by the SLIM
protocol.

For detailed instructions on compiling a protobuf file to obtain the necessary
slimrpc stub code, see the [slimrpc compiler README](../../slimrpc-compiler/README.md).

## SLIM naming in slimrpc

In slimrpc, each service and its individual RPC handlers are assigned a SLIM name,
facilitating efficient message routing and processing. For slimrpc, service
methods are invoked using the format:

```
{package-name}.{service-name}/{method-name}
```

For example, a service `example_service.Test` with method `ExampleUnaryUnary` would
use the name `example_service.Test/ExampleUnaryUnary`.

The slimrpc package manages all the underlying SLIM communication. Application
developers only need to implement the specific functions that will be invoked
when a message arrives for a defined RPC method.

## C# Setup

### Prerequisites

- .NET 8.0 or higher
- [buf](https://buf.build/docs/installation) CLI
- [protoc-gen-slimrpc-csharp](../../slimrpc-compiler/README.md) plugin (build from source)

### Code Generation

Add the `Agntcy.Slim` and `Agntcy.Slim.SlimRpc` NuGet packages to your project.
Then configure `buf.gen.yaml` to generate C# slimrpc stubs:

```yaml
version: v2
managed:
  enabled: true
plugins:
  # Generates standard .pb.cs files for C#
  - remote: buf.build/protocolbuffers/csharp
    out: Generated
    opt: base_namespace=ExampleService

  # Generates _slimrpc.cs files (client stubs and server handlers)
  - local: /path/to/target/release/protoc-gen-slimrpc-csharp
    out: Generated
    opt: base_namespace=ExampleService,types_namespace=ExampleService
```

Run `buf generate` to produce the generated code. For the example project in this
repository, use:

```bash
task slimrpc:generate-proto
```

## Creating a Channel and Server

### Server

Create an RPC server using `SlimRpcServerFactory.CreateServer`:

```csharp
using Agntcy.Slim;
using Agntcy.Slim.SlimRpc;

// After creating app and connecting to SLIM...
var localName = SlimName.Parse("agntcy/grpc/server");
var server = SlimRpcServerFactory.CreateServer(app, localName, connectionId);

// Register your service implementation
TestServerRegistration.RegisterTestServer(server, new MyTestServerImpl());

await server.ServeAsync();
```

### Client

Create an RPC channel using `SlimRpcChannelFactory.CreateChannel` and pass it to
the generated client:

```csharp
using Agntcy.Slim;
using Agntcy.Slim.SlimRpc;

// After creating app and connecting to SLIM...
var remoteName = SlimName.Parse("agntcy/grpc/server");
var channel = SlimRpcChannelFactory.CreateChannel(app, remoteName, connectionId);
var client = new TestClient(channel);

// Make RPC calls
var request = new ExampleRequest { ExampleString = "hello", ExampleInteger = 1 };
var response = await client.ExampleUnaryUnaryAsync(request);
```

## RPC Patterns

The generated client supports all four gRPC streaming patterns:

- **Unary-Unary**: `ExampleUnaryUnaryAsync(request)` — single request, single response
- **Unary-Stream**: `ExampleUnaryStreamAsync(request)` — single request, streaming responses
- **Stream-Unary**: `ExampleStreamUnaryAsync(requestStream)` — streaming requests, single response
- **Stream-Stream**: `ExampleStreamStreamAsync(requestStream)` — bidirectional streaming

All client methods accept optional `timeout`, `metadata`, and `cancellationToken` parameters.

## Example

A complete working example is available in `Slim.Examples.SlimRpc`. To run it:

1. Start a SLIM server: `cd data-plane && cargo run --bin slim -- --config ./config/base/server-config.yaml`
2. Generate proto code: `task slimrpc:generate-proto`
3. Run the server: `dotnet run --project Slim.Examples.SlimRpc -- --mode server`
4. Run the client (in another terminal): `dotnet run --project Slim.Examples.SlimRpc -- --mode client`
