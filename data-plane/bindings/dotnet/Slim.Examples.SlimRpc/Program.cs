// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

using Agntcy.Slim;
using Agntcy.Slim.Examples.Common;
using Agntcy.Slim.SlimRpc;
using ExampleService;
using System.CommandLine;

namespace Slim.Examples.SlimRpc;

class Program
{
    static async Task<int> Main(string[] args)
    {
        var modeOption = new Option<string>(
            name: "--mode",
            description: "Run mode: 'server' or 'client'")
        {
            IsRequired = true
        };

        var serverOption = new Option<string>(
            name: "--server",
            getDefaultValue: () => CommonHelpers.DefaultServerEndpoint,
            description: "SLIM server endpoint");

        var sharedSecretOption = new Option<string>(
            name: "--shared-secret",
            getDefaultValue: () => CommonHelpers.DefaultSharedSecret,
            description: "Shared secret (min 32 chars)");

        var rootCommand = new RootCommand("SLIM slimrpc Example");
        rootCommand.AddOption(modeOption);
        rootCommand.AddOption(serverOption);
        rootCommand.AddOption(sharedSecretOption);

        rootCommand.SetHandler(async (context) =>
        {
            var mode = context.ParseResult.GetValueForOption(modeOption)!;
            var server = context.ParseResult.GetValueForOption(serverOption)!;
            var sharedSecret = context.ParseResult.GetValueForOption(sharedSecretOption)!;

            if (mode.Equals("server", StringComparison.OrdinalIgnoreCase))
            {
                await RunServerAsync(server, sharedSecret);
            }
            else if (mode.Equals("client", StringComparison.OrdinalIgnoreCase))
            {
                await RunClientAsync(server, sharedSecret);
            }
            else
            {
                Console.Error.WriteLine($"Unknown mode: {mode}. Use 'server' or 'client'.");
                context.ExitCode = 1;
            }
        });

        return await rootCommand.InvokeAsync(args);
    }

    static async Task RunServerAsync(string serverEndpoint, string sharedSecret)
    {
        var (app, connId) = CommonHelpers.CreateAndConnectApp("agntcy/grpc/server", serverEndpoint, sharedSecret);

        using var localName = SlimName.Parse("agntcy/grpc/server");
        var slimServer = SlimRpcServerFactory.CreateServer(app, localName, connId);

        var impl = new TestServerImpl();
        TestServerRegistration.RegisterTestServer(slimServer, impl);

        Console.WriteLine("Server starting... (Ctrl+C to stop)");
        Console.CancelKeyPress += (_, _) => Console.WriteLine("Server stopping...");

        try
        {
            await slimServer.ServeAsync();
        }
        catch (OperationCanceledException)
        {
            Console.WriteLine("Server stopped.");
        }
    }

    static async Task RunClientAsync(string serverEndpoint, string sharedSecret)
    {
        var (app, connId) = CommonHelpers.CreateAndConnectApp("agntcy/grpc/client", serverEndpoint, sharedSecret);

        using var remoteName = SlimName.Parse("agntcy/grpc/server");
        var channel = SlimRpcChannelFactory.CreateChannel(app, remoteName, connId);
        var client = new TestClient(channel);

        var request = new ExampleRequest { ExampleString = "hello", ExampleInteger = 1 };

        Console.WriteLine("=== Unary-Unary ===");
        try
        {
            var response = await client.ExampleUnaryUnaryAsync(request);
            Console.WriteLine($"Response: {response}");
        }
        catch (Exception ex)
        {
            Console.WriteLine($"Unary-Unary failed: {ex.Message}");
        }

        Console.WriteLine("\n=== Unary-Stream ===");
        try
        {
            await foreach (var resp in client.ExampleUnaryStreamAsync(request))
            {
                Console.WriteLine($"  Stream response: {resp}");
            }
        }
        catch (Exception ex)
        {
            Console.WriteLine($"Unary-Stream failed: {ex.Message}");
        }

        Console.WriteLine("\n=== Stream-Unary ===");
        async IAsyncEnumerable<ExampleRequest> StreamRequests()
        {
            for (var i = 0; i < 10; i++)
                yield return new ExampleRequest { ExampleInteger = i, ExampleString = $"Request {i}" };
        }
        try
        {
            var streamUnaryResp = await client.ExampleStreamUnaryAsync(StreamRequests());
            Console.WriteLine($"Stream-Unary: {streamUnaryResp}");
        }
        catch (Exception ex)
        {
            Console.WriteLine($"Stream-Unary failed: {ex.Message}");
        }

        Console.WriteLine("\n=== Stream-Stream ===");
        try
        {
            await foreach (var r in client.ExampleStreamStreamAsync(StreamRequests()))
            {
                Console.WriteLine($"  Stream-Stream: {r}");
            }
        }
        catch (Exception ex)
        {
            Console.WriteLine($"Stream-Stream failed: {ex.Message}");
        }

        Console.WriteLine("\nDone.");
    }
}

/// <summary>
/// Test server implementation.
/// </summary>
class TestServerImpl : ITestServer
{
    public Task<ExampleResponse> ExampleUnaryUnary(ExampleRequest request, SlimRpcContext context)
    {
        Console.WriteLine($"Received unary-unary: {request}");
        return Task.FromResult(new ExampleResponse
        {
            ExampleInteger = 1,
            ExampleString = "Hello, World!"
        });
    }

    public async IAsyncEnumerable<ExampleResponse> ExampleUnaryStream(ExampleRequest request, SlimRpcContext context)
    {
        Console.WriteLine($"Received unary-stream: {request}");
        for (var i = 0; i < 5; i++)
        {
            yield return new ExampleResponse
            {
                ExampleInteger = i,
                ExampleString = $"Response {i}"
            };
        }
    }

    public async IAsyncEnumerable<ExampleResponse> ExampleUnaryStreamTwo(ExampleRequest request, SlimRpcContext context)
    {
        await foreach (var r in ExampleUnaryStream(request, context))
        {
            yield return r;
        }
    }

    public async Task<ExampleResponse> ExampleStreamUnary(IAsyncEnumerable<ExampleRequest> requestStream, SlimRpcContext context)
    {
        var sum = 0L;
        var count = 0;
        await foreach (var req in requestStream)
        {
            sum += req.ExampleInteger;
            count++;
        }
        return new ExampleResponse
        {
            ExampleInteger = sum,
            ExampleString = $"Received {count} messages"
        };
    }

    public async IAsyncEnumerable<ExampleResponse> ExampleStreamStream(IAsyncEnumerable<ExampleRequest> requestStream, SlimRpcContext context)
    {
        await foreach (var req in requestStream)
        {
            yield return new ExampleResponse
            {
                ExampleInteger = req.ExampleInteger * 100,
                ExampleString = $"Echo: {req.ExampleString}"
            };
        }
    }
}
