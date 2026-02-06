// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

using Agntcy.Slim;
using Agntcy.Slim.Examples.Common;
using System.CommandLine;

namespace Agntcy.Slim.Examples.PointToPoint;

class Program
{
    static async Task<int> Main(string[] args)
    {
        var localOption = new Option<string>(
            name: "--local",
            description: "Local ID (org/namespace/app) - required")
        {
            IsRequired = true
        };

        var remoteOption = new Option<string?>(
            name: "--remote",
            description: "Remote ID (org/namespace/app)");

        var serverOption = new Option<string>(
            name: "--server",
            getDefaultValue: () => CommonHelpers.DefaultServerEndpoint,
            description: "SLIM server endpoint");

        var messageOption = new Option<string?>(
            name: "--message",
            description: "Message to send (sender mode)");

        var iterationsOption = new Option<int>(
            name: "--iterations",
            getDefaultValue: () => 10,
            description: "Number of messages to send");

        var sharedSecretOption = new Option<string>(
            name: "--shared-secret",
            getDefaultValue: () => CommonHelpers.DefaultSharedSecret,
            description: "Shared secret (min 32 chars)");

        var enableMlsOption = new Option<bool>(
            name: "--enable-mls",
            getDefaultValue: () => false,
            description: "Enable MLS encryption");

        var rootCommand = new RootCommand("SLIM Point-to-Point Example");
        rootCommand.AddOption(localOption);
        rootCommand.AddOption(remoteOption);
        rootCommand.AddOption(serverOption);
        rootCommand.AddOption(messageOption);
        rootCommand.AddOption(iterationsOption);
        rootCommand.AddOption(sharedSecretOption);
        rootCommand.AddOption(enableMlsOption);

        rootCommand.SetHandler(async (context) =>
        {
            var local = context.ParseResult.GetValueForOption(localOption)!;
            var remote = context.ParseResult.GetValueForOption(remoteOption);
            var server = context.ParseResult.GetValueForOption(serverOption)!;
            var message = context.ParseResult.GetValueForOption(messageOption);
            var iterations = context.ParseResult.GetValueForOption(iterationsOption);
            var sharedSecret = context.ParseResult.GetValueForOption(sharedSecretOption)!;
            var enableMls = context.ParseResult.GetValueForOption(enableMlsOption);

            // Create and connect app
            var (app, connId) = CommonHelpers.CreateAndConnectApp(local, server, sharedSecret);

            var instance = app.Id;
            Console.WriteLine($"[{instance}] Created app");
            Console.WriteLine($"[{instance}] Connected to {server} (conn ID: {connId})");

            // Run sender or receiver mode
            if (!string.IsNullOrEmpty(message) && !string.IsNullOrEmpty(remote))
            {
                await RunSender(app, connId, remote, message, iterations, enableMls, instance);
            }
            else if (!string.IsNullOrEmpty(message))
            {
                Console.WriteLine("Error: --remote required when --message specified");
                context.ExitCode = 1;
            }
            else
            {
                await RunReceiver(app, instance);
            }

            app.Destroy();
        });

        return await rootCommand.InvokeAsync(args);
    }

    static async Task RunSender(SlimApp app, ulong connId, string remote, string message, 
        int iterations, bool enableMls, ulong instance)
    {
        using var remoteName = CommonHelpers.SplitId(remote);

        // Set route to remote via the server connection
        app.SetRoute(remoteName, connId);
        Console.WriteLine($"[{instance}] Route set to {remote} via connection {connId}");

        var config = new SlimSessionConfig
        {
            SessionType = SlimSessionType.PointToPoint,
            EnableMls = enableMls
        };

        Console.WriteLine($"[{instance}] Creating session to {remote}...");
        using var session = await app.CreateSessionAsync(remoteName, config);

        // Give session a moment to establish
        await Task.Delay(100);

        Console.WriteLine($"[{instance}] Session created");

        for (int i = 0; i < iterations; i++)
        {
            try
            {
                await session.PublishAsync(message);
                Console.WriteLine($"[{instance}] Sent message '{message}' - {i + 1}/{iterations}");

                // Wait for reply
                var msg = await session.GetMessageAsync(TimeSpan.FromSeconds(5));
                Console.WriteLine($"[{instance}] Received reply '{msg.Text}' - {i + 1}/{iterations}");
                
                await Task.Delay(1000);
            }
            catch (Exception)
            {
                Console.WriteLine($"[{instance}] Error sending/receiving message {i + 1}/{iterations}");
        }
    }

    // Note: session.Dispose() is automatically called by 'using var' at end of method
}

    static async Task RunReceiver(SlimApp app, ulong instance)
    {
        Console.WriteLine($"[{instance}] Waiting for incoming sessions...");

        while (true)
        {
            try
            {
                var session = await app.ListenForSessionAsync();
                Console.WriteLine($"[{instance}] New session established!");
                
                // Handle session in background
                _ = Task.Run(() => HandleSession(app, session, instance));
            }
            catch (Exception)
            {
                Console.WriteLine($"[{instance}] Timeout waiting for session, retrying...");
            }
        }
    }

    static async Task HandleSession(SlimApp app, SlimSession session, ulong instance)
    {
        try
        {
            using (session)
            {
                while (true)
                {
                    try
                    {
                        var msg = await session.GetMessageAsync(TimeSpan.FromSeconds(60));
                        Console.WriteLine($"[{instance}] Received: {msg.Text}");

                        var reply = $"{msg.Text} from {instance}";
                        await session.ReplyAsync(msg, reply);
                        Console.WriteLine($"[{instance}] Replied: {reply}");
                    }
                    catch (Exception ex)
                    {
                        Console.WriteLine($"[{instance}] Session ended: {ex.Message}");
                        break;
                    }
            }
        }

        // Note: session.Dispose() is automatically called by 'using' block
        Console.WriteLine($"[{instance}] Session closed");
    }
    catch (Exception ex)
    {
        Console.WriteLine($"[{instance}] Error handling session: {ex.Message}");
    }
    }
}
