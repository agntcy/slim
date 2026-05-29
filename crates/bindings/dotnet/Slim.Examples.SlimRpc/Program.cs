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
            description: "Run mode: 'server', 'client', or 'group-client'")
        {
            IsRequired = true
        };

        var serverOption = new Option<string>(
            name: "--server",
            getDefaultValue: () => CommonHelpers.GetServerEndpoint(),
            description: "SLIM server endpoint");

        var sharedSecretOption = new Option<string>(
            name: "--shared-secret",
            getDefaultValue: () => Environment.GetEnvironmentVariable("SLIM_SHARED_SECRET")
                ?? CommonHelpers.DefaultSharedSecret,
            description: "Shared secret (min 32 chars). Default: SLIM_SHARED_SECRET env var or demo value.");

        var instanceOption = new Option<string>(
            name: "--instance",
            getDefaultValue: () => "server",
            description: "Instance name used as the SLIM app name (default: server). Use server1/server2 for group example.");

        var rootCommand = new RootCommand("SLIM slimrpc Example");
        rootCommand.AddOption(modeOption);
        rootCommand.AddOption(serverOption);
        rootCommand.AddOption(sharedSecretOption);
        rootCommand.AddOption(instanceOption);

        var serversOption = new Option<string>(
            name: "--servers",
            getDefaultValue: () => "server1,server2",
            description: "Comma-separated server instance names for group-client mode");
        rootCommand.AddOption(serversOption);

        rootCommand.SetHandler(async (context) =>
        {
            var mode = context.ParseResult.GetValueForOption(modeOption)!;
            var server = context.ParseResult.GetValueForOption(serverOption)!;
            var sharedSecret = context.ParseResult.GetValueForOption(sharedSecretOption)!;
            var instance = context.ParseResult.GetValueForOption(instanceOption)!;
            var servers = context.ParseResult.GetValueForOption(serversOption)!;

            if (mode.Equals("server", StringComparison.OrdinalIgnoreCase))
            {
                await RunServerAsync(server, sharedSecret, instance);
            }
            else if (mode.Equals("client", StringComparison.OrdinalIgnoreCase))
            {
                await RunClientAsync(server, sharedSecret);
            }
            else if (mode.Equals("group-client", StringComparison.OrdinalIgnoreCase))
            {
                await RunGroupClientAsync(server, sharedSecret, servers);
            }
            else
            {
                Console.Error.WriteLine($"Unknown mode: {mode}. Use 'server', 'client', or 'group-client'.");
                context.ExitCode = 1;
            }
        });

        return await rootCommand.InvokeAsync(args);
    }

    static async Task RunServerAsync(string serverEndpoint, string sharedSecret, string instance)
    {
        var appId = $"agntcy/grpc/{instance}";
        var (app, connId) = CommonHelpers.CreateAndConnectApp(appId, serverEndpoint, sharedSecret);
        using (app)
        {
            using var localName = SlimName.Parse(appId);
            using var slimServer = SlimRpcServerFactory.CreateServer(app, localName, connId);

            var impl = new TestServerImpl();
            TestServerRegistration.RegisterTestServer(slimServer, impl);

            Console.WriteLine("SLIM_RPC_SERVER_READY");
            Console.WriteLine($"Server '{instance}' starting... (Ctrl+C to stop)");
            var serveTask = slimServer.ServeAsync();
            Console.CancelKeyPress += (_, e) =>
            {
                e.Cancel = true;
                _ = slimServer.ShutdownAsync();
            };

            try
            {
                await serveTask;
            }
            catch (OperationCanceledException)
            {
                Console.WriteLine("Server stopped.");
            }
        }
    }

    static async Task RunClientAsync(string serverEndpoint, string sharedSecret)
    {
        var (app, connId) = CommonHelpers.CreateAndConnectApp("agntcy/grpc/client", serverEndpoint, sharedSecret);
        using (app)
        {
            using var remoteName = SlimName.Parse("agntcy/grpc/server");
            using var channel = SlimRpcChannelFactory.CreateChannel(app, remoteName, connId);
            var client = new TestClient(channel);

            var request = new ExampleRequest { ExampleString = "hello", ExampleInteger = 1 };

            Console.WriteLine("SLIM_RPC_CLIENT_STARTED");
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
                await Task.Yield();
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

            Console.WriteLine("SLIM_RPC_CLIENT_DONE");
            Console.WriteLine("\nDone.");
        }
    }
    static async Task RunGroupClientAsync(string serverEndpoint, string sharedSecret, string servers)
    {
        var (app, connId) = CommonHelpers.CreateAndConnectApp("agntcy/grpc/client", serverEndpoint, sharedSecret);
        using (app)
        {
            // Group channel targeting multiple server instances
            var serverInstances = servers.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
            var serverNames = serverInstances.Select(s => SlimName.Parse($"agntcy/grpc/{s}")).ToArray();

            try
            {
                using var channel = SlimRpcChannelFactory.CreateGroupChannel(app, serverNames, connId);
                var client = new TestGroupClient(channel);

                var request = new ExampleRequest { ExampleString = "hello", ExampleInteger = 1 };

                Console.WriteLine("SLIM_RPC_GROUP_CLIENT_STARTED");
                Console.WriteLine("=== Multicast Unary-Unary ===");
                try
                {
                    await foreach (var item in client.ExampleUnaryUnaryAsync(request, TimeSpan.FromSeconds(5)))
                    {
                        Console.WriteLine($"  [{item.Context}] {item.Value}");
                    }
                }
                catch (Exception ex)
                {
                    Console.WriteLine($"Multicast Unary-Unary failed: {ex.Message}");
                }

                Console.WriteLine("\n=== Multicast Unary-Stream ===");
                try
                {
                    await foreach (var item in client.ExampleUnaryStreamAsync(request, TimeSpan.FromSeconds(5)))
                    {
                        Console.WriteLine($"  [{item.Context}] {item.Value}");
                    }
                }
                catch (Exception ex)
                {
                    Console.WriteLine($"Multicast Unary-Stream failed: {ex.Message}");
                }

                Console.WriteLine("\n=== Multicast Stream-Unary ===");
                async IAsyncEnumerable<ExampleRequest> StreamRequests()
                {
                    await Task.Yield();
                    for (var i = 0; i < 3; i++)
                        yield return new ExampleRequest { ExampleInteger = i, ExampleString = $"item {i}" };
                }
                try
                {
                    await foreach (var item in client.ExampleStreamUnaryAsync(StreamRequests(), TimeSpan.FromSeconds(5)))
                    {
                        Console.WriteLine($"  [{item.Context}] {item.Value}");
                    }
                }
                catch (Exception ex)
                {
                    Console.WriteLine($"Multicast Stream-Unary failed: {ex.Message}");
                }

                Console.WriteLine("\n=== Multicast Stream-Stream ===");
                try
                {
                    await foreach (var item in client.ExampleStreamStreamAsync(StreamRequests(), TimeSpan.FromSeconds(5)))
                    {
                        Console.WriteLine($"  [{item.Context}] {item.Value}");
                    }
                }
                catch (Exception ex)
                {
                    Console.WriteLine($"Multicast Stream-Stream failed: {ex.Message}");
                }

                Console.WriteLine("SLIM_RPC_GROUP_CLIENT_DONE");
                Console.WriteLine("\nDone.");
            }
            finally
            {
                foreach (var name in serverNames) name.Dispose();
            }
        }
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
        await Task.Yield();
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
