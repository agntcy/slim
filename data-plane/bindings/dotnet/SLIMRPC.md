# slimrpc (SLIM Remote Procedure Call)

slimrpc, or SLIM Remote Procedure Call, is a mechanism designed to enable Protocol
Buffers (protobuf) RPC over SLIM (Secure Low-latency Inter-process Messaging).
This is analogous to gRPC, which leverages HTTP/2 as its underlying transport
layer for protobuf RPC.

A key advantage of slimrpc lies in its ability to seamlessly integrate SLIM as the
transport protocol for inter-application message exchange. This significantly
simplifies development: a protobuf file can be compiled to generate code that
utilizes SLIM for communication. Application developers can then interact with
the generated code much like they would with standard gRPC, while benefiting
from the inherent security features and efficiency provided by the SLIM
protocol.

This document provides a guide to using slimrpc with the C# bindings. For detailed
instructions on compiling a protobuf file to obtain the necessary slimrpc stub
code, please refer to the [slimrpc compiler README](../../slimrpc-compiler/README.md).

## SLIM naming in slimrpc

In slimrpc, each service and its individual RPC handlers are assigned a SLIM name,
facilitating efficient message routing and processing. Service methods are invoked
using the format:

```
{package-name}.{service-name}/{method-name}
```

For example, with `example_service.Test` service, method names would be:

```
example_service.Test/ExampleUnaryUnary
example_service.Test/ExampleUnaryStream
example_service.Test/ExampleStreamUnary
example_service.Test/ExampleStreamStream
```

## C# Generated Code

The C# slimrpc plugin (`protoc-gen-slimrpc-csharp`) generates:

- **Client stub** (`TestClient`): Async methods calling `Channel.CallUnaryAsync`, `CallUnaryStreamAsync`, etc.
- **Server interface** (`ITestServer`): Method signatures for implementing the service
- **Registration** (`TestServerRegistration.RegisterTestServer`): Registers handlers with the SLIM server

Output file naming: `example.proto` → `example_slimrpc.cs`

## Example

The [Slim.Examples.SlimRpc](Slim.Examples.SlimRpc/) project demonstrates a complete
client-server slimrpc implementation.

### Prerequisites

1. **Running SLIM server** on `localhost:46357` (default). Start it with:
   ```bash
   cd data-plane && cargo run --bin slim -- --config ./config/base/server-config.yaml
   ```

2. Build the C# slimrpc plugin:
   ```bash
   cd data-plane/slimrpc-compiler
   cargo build --release --bin protoc-gen-slimrpc-csharp
   ```

3. Install [buf](https://buf.build/docs/installation) for code generation.

### Generate Code

```bash
cd data-plane/bindings/dotnet
task slimrpc:generate-proto
```

Or manually:

```bash
cd Slim.Examples.SlimRpc
buf generate
```

### Run Server and Client

Ensure the SLIM server is running first (see Prerequisites). Then:

```bash
# Terminal 1: Start the slimrpc server
dotnet run --project Slim.Examples.SlimRpc -- --mode server

# Terminal 2: Run the slimrpc client
dotnet run --project Slim.Examples.SlimRpc -- --mode client
```

### Server Implementation

Implement `ITestServer` and register with the server:

```csharp
var impl = new TestServerImpl();
TestServerRegistration.RegisterTestServer(slimServer, impl);
```

### Client Usage

Create a channel and use the generated client:

```csharp
var channel = SlimRpcChannelFactory.CreateChannel(app, remoteName, connId);
var client = new TestClient(channel);

var request = new ExampleRequest { ExampleString = "hello", ExampleInteger = 1 };
var response = await client.ExampleUnaryUnaryAsync(request);
```

## Dependencies

- **Agntcy.Slim**: Core SLIM bindings (Channel, Server, Context)
- **Agntcy.Slim.SlimRpc**: Helper library (SlimRpcContext, SlimRpcStreams, channel/server factories)
- **Google.Protobuf**: Protobuf message types
