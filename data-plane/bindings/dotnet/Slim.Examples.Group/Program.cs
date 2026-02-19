// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

using Agntcy.Slim;
using Agntcy.Slim.Examples.Common;
using System.CommandLine;

namespace Agntcy.Slim.Examples.Group;

class Program
{
    // ANSI color codes for terminal output
    const string ColorReset = "\u001b[0m";
    const string ColorCyan = "\u001b[96m";
    const string ColorYellow = "\u001b[93m";
    const string ColorGreen = "\u001b[92m";

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
            description: "Remote ID (org/namespace/channel) - for group topic");

        var serverOption = new Option<string>(
            name: "--server",
            getDefaultValue: () => CommonHelpers.DefaultServerEndpoint,
            description: "SLIM server endpoint");

        var sharedSecretOption = new Option<string>(
            name: "--shared-secret",
            getDefaultValue: () => CommonHelpers.DefaultSharedSecret,
            description: "Shared secret (min 32 chars)");

        var enableMlsOption = new Option<bool>(
            name: "--enable-mls",
            getDefaultValue: () => false,
            description: "Enable MLS encryption");

        var invitesOption = new Option<string?>(
            name: "--invites",
            description: "Comma-separated list of participant IDs to invite (moderator mode)");

        var rootCommand = new RootCommand("SLIM Group Messaging Example");
        rootCommand.AddOption(localOption);
        rootCommand.AddOption(remoteOption);
        rootCommand.AddOption(serverOption);
        rootCommand.AddOption(sharedSecretOption);
        rootCommand.AddOption(enableMlsOption);
        rootCommand.AddOption(invitesOption);

        rootCommand.SetHandler(async (context) =>
        {
            var local = context.ParseResult.GetValueForOption(localOption)!;
            var remote = context.ParseResult.GetValueForOption(remoteOption);
            var server = context.ParseResult.GetValueForOption(serverOption)!;
            var sharedSecret = context.ParseResult.GetValueForOption(sharedSecretOption)!;
            var enableMls = context.ParseResult.GetValueForOption(enableMlsOption);
            var invitesStr = context.ParseResult.GetValueForOption(invitesOption);

            // Parse invites
            var inviteList = new List<string>();
            if (!string.IsNullOrEmpty(invitesStr))
            {
                inviteList = invitesStr.Split(',')
                    .Select(s => s.Trim())
                    .Where(s => !string.IsNullOrEmpty(s))
                    .ToList();
            }

            // Determine mode
            bool isModerator = !string.IsNullOrEmpty(remote) && inviteList.Count > 0;

            // Create and connect app
            var (app, connId) = CommonHelpers.CreateAndConnectApp(local, server, sharedSecret);

            var instance = app.Id;
            Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Created app");
            Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Connected to {server} (conn ID: {connId})");

            // Run in appropriate mode
            if (isModerator)
            {
                await RunModerator(app, connId, remote!, inviteList, enableMls, instance);
            }
            else
            {
                await RunParticipant(app, instance);
            }

            app.Destroy();
        });

        return await rootCommand.InvokeAsync(args);
    }

    static async Task RunModerator(SlimApp app, ulong connId, string remote, 
        List<string> invites, bool enableMls, ulong instance)
    {
        // Parse remote channel name
        using var channelName = SlimName.Parse(remote);

        Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Creating group session as moderator for channel: {remote}");

        // Create multicast session
        var config = new SlimSessionConfig
        {
            SessionType = SlimSessionType.Group,
            EnableMls = enableMls,
            MaxRetries = 5,
            RetryInterval = TimeSpan.FromSeconds(5),
            Metadata = new Dictionary<string, string>()
        };

        using var session = await app.CreateSessionAsync(channelName, config);

        // Give session a moment to establish
        await Task.Delay(100);
        Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Group session created");

        // Invite each participant
        foreach (var inviteId in invites)
        {
            try
            {
                using var inviteName = SlimName.Parse(inviteId);

                // Set route for invitee
                app.SetRoute(inviteName, connId);

                // Send invite
                session.Invite(inviteName);

                Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Invited {inviteId} to the group");
            }
            catch (Exception ex)
            {
                Console.WriteLine($"Failed to invite {inviteId}: {ex.Message}");
            }
        }

        // Run message loops
        await RunMessageLoops(app, session, channelName, instance);
        
        // Note: session.Dispose() is automatically called by 'using var' at end of method
    }

    static async Task RunParticipant(SlimApp app, ulong instance)
    {
        Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Waiting for incoming group session invitation...");

        // Wait for incoming session
        using var session = await app.ListenForSessionAsync(TimeSpan.FromSeconds(60));
        using var channelName = session.Destination;

        Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Joined group session for channel: {channelName}");

        // Run message loops
        await RunMessageLoops(app, session, channelName, instance);
        
        // Note: session.Dispose() is automatically called by 'using var' at end of method
    }

    static async Task RunMessageLoops(SlimApp app, SlimSession session, 
        SlimName channelName, ulong instance)
    {
        using var cts = new CancellationTokenSource();
        var sourceName = session.Source;

        // Start receive task
        var receiveTask = Task.Run(() => ReceiveLoop(session, sourceName, instance, cts.Token));

        // Start keyboard input task
        var keyboardTask = Task.Run(() => KeyboardLoop(session, sourceName, channelName, instance, cts.Token));

        // Wait for either task to complete
        await Task.WhenAny(receiveTask, keyboardTask);
        
        // Cancel remaining tasks
        cts.Cancel();
        
        // Wait for both to finish
        try
        {
            await Task.WhenAll(receiveTask, keyboardTask);
        }
        catch (OperationCanceledException)
        {
            // Expected when cancelling
        }
    }

    static async Task ReceiveLoop(SlimSession session, SlimName sourceName, 
        ulong instance, CancellationToken cancellationToken)
    {
        while (!cancellationToken.IsCancellationRequested)
        {
            try
            {
                var msg = await session.GetMessageAsync(TimeSpan.FromSeconds(1));
                
                // Display received message
                Console.WriteLine($"\n{ColorYellow}[Message] > {msg.Text}{ColorReset}");

                // Re-print prompt
                Console.Write($"{ColorGreen}{sourceName} > {ColorReset}");
            }
            catch (Exception ex) when (ex.Message.Contains("timeout") || ex.Message.Contains("timed out"))
            {
                // Timeout is expected, continue
                continue;
            }
            catch (Exception ex)
            {
                Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Session ended: {ex.Message}");
                break;
            }
        }
    }

    static async Task KeyboardLoop(SlimSession session, SlimName sourceName, 
        SlimName channelName, ulong instance, CancellationToken cancellationToken)
    {
        Console.WriteLine($"\n{ColorCyan}[{instance}]{ColorReset} Welcome to the group {channelName}!");
        
        var participants = session.GetParticipants();
        Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Participants in the group: {participants.Count}");
        foreach (var participant in participants)
        {
            Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Participant: {participant}");
        }

        Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Type a message and press Enter to send, or 'exit'/'quit' to leave.\n");
        Console.Write($"{ColorGreen}{sourceName} > {ColorReset}");

        while (!cancellationToken.IsCancellationRequested)
        {
            // Read input with cancellation support
            var input = await Task.Run(() => Console.ReadLine(), cancellationToken);
            
            if (string.IsNullOrWhiteSpace(input))
            {
                Console.Write($"{ColorGreen}{sourceName} > {ColorReset}");
                continue;
            }

            // Check for exit commands
            if (input.ToLower() is "exit" or "quit")
            {
                Console.WriteLine($"\n{ColorCyan}[{instance}]{ColorReset} Leaving group...");
                break;
            }

            // Publish message to group
            try
            {
                await session.PublishAsync(input);
                Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Sent to group");
            }
            catch (Exception ex)
            {
                Console.WriteLine($"{ColorCyan}[{instance}]{ColorReset} Error sending message: {ex.Message}");
            }

            Console.Write($"{ColorGreen}{sourceName} > {ColorReset}");
        }
    }
}
