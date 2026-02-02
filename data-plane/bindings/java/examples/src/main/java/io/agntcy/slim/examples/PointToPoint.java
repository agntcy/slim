// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples;

import io.agntcy.slim.examples.common.*;
import io.agntcy.slim.bindings.*;

import java.time.Duration;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Point-to-point messaging example using Java CompletableFuture.
 *
 * Demonstrates:
 * - Creating SLIM apps with shared secret authentication
 * - Connecting to a SLIM server
 * - Creating point-to-point sessions
 * - Sending and receiving messages with async operations
 *
 * Usage:
 * Receiver: --local org/namespace/alice
 * Sender: --local org/namespace/bob --remote org/namespace/alice --message
 * "Hello"
 */
public class PointToPoint {

    public static void main(String[] args) {
        try {
            // Parse arguments
            Config.ArgParser parser = new Config.ArgParser(args);
            Config.PointToPointConfig config = new Config.PointToPointConfig(
                    parser.getRequiredOption("local"));
            config.remote = parser.getOption("remote", null);
            config.server = parser.getOption("server", Common.DEFAULT_SERVER_ENDPOINT);
            config.message = parser.getOption("message", null);
            config.iterations = parser.getIntOption("iterations", 10);
            config.sharedSecret = parser.getOption("shared-secret", Common.DEFAULT_SHARED_SECRET);
            config.enableMls = parser.hasFlag("enable-mls");

            // Create and connect app
            Common.AppConnection connection = Common.createAndConnectApp(
                    config.local, config.server, config.sharedSecret);
            App app = connection.app;
            long connId = connection.connectionId;

            long instanceId = app.id();
            System.out.println(Colors.instancePrefix(instanceId) + "✅ Created app");
            System.out.println(Colors.instancePrefix(instanceId) +
                    "🔌 Connected to " + config.server + " (conn ID: " + connId + ")");

            // Run sender or receiver mode
            if (config.isSender() && config.remote != null) {
                runSender(app, connId, config, instanceId);
            } else if (config.isSender()) {
                System.err.println("Error: --remote required when --message specified");
                System.exit(1);
            } else {
                runReceiver(app, instanceId);
            }

            app.close();

        } catch (IllegalArgumentException e) {
            System.err.println("Configuration error: " + e.getMessage());
            printUsage();
            System.exit(1);
        } catch (Exception e) {
            System.err.println("Error: " + e.getMessage());
            e.printStackTrace();
            System.exit(1);
        }
    }

    private static void runSender(App app, long connId, Config.PointToPointConfig config, long instanceId)
            throws Exception {
        Name remoteName = Common.splitId(config.remote);

        // Set route to remote via the server connection
        app.setRouteAsync(remoteName, connId).get();
        System.out.println(Colors.instancePrefix(instanceId) +
                "📍 Route set to " + config.remote + " via connection " + connId);

        // Create session configuration
        SessionConfig sessionConfig = new SessionConfig(
                SessionType.POINT_TO_POINT,
                config.enableMls,
                null, // maxRetries
                null, // interval
                Map.of() // metadata - use empty map instead of null
        );

        System.out.println(Colors.instancePrefix(instanceId) +
                "🔍 Creating session to " + config.remote + "...");

        Session session = app.createSessionAndWaitAsync(sessionConfig, remoteName).get();

        // Give session a moment to establish
        Thread.sleep(100);

        System.out.println(Colors.instancePrefix(instanceId) + "📡 Session created");

        // Send messages
        for (int i = 0; i < config.iterations; i++) {
            try {
                session.publishAndWaitAsync(config.message.getBytes(), null, null).get();
                System.out.println(Colors.instancePrefix(instanceId) +
                        "📤 Sent message '" + config.message + "' - " + (i + 1) + "/" + config.iterations);

                // Wait for reply
                Duration timeout = Duration.ofSeconds(5);
                ReceivedMessage msg = session.getMessageAsync(timeout).get();
                System.out.println(Colors.instancePrefix(instanceId) +
                        "📥 Received reply '" + new String(msg.payload()) + "' - " + (i + 1) + "/"
                        + config.iterations);

                Thread.sleep(1000);
            } catch (Exception e) {
                System.err.println(Colors.instancePrefix(instanceId) +
                        "❌ Error with message " + (i + 1) + "/" + config.iterations + ": " + e.getMessage());
            }
        }

        // Clean up
        app.deleteSessionAndWaitAsync(session).get();
    }

    private static void runReceiver(App app, long instanceId) throws Exception {
        System.out.println(Colors.instancePrefix(instanceId) + "👂 Waiting for incoming sessions...");

        ExecutorService executor = Executors.newCachedThreadPool();

        while (true) {
            try {
                Session session = app.listenForSessionAsync(null).get();
                System.out.println(Colors.instancePrefix(instanceId) + "🎉 New session established!");

                // Handle session in background
                executor.submit(() -> handleSession(app, session, instanceId));
            } catch (Exception e) {
                System.out.println(Colors.instancePrefix(instanceId) +
                        "⏱️  Timeout waiting for session, retrying...");
            }
        }
    }

    private static void handleSession(App app, Session session, long instanceId) {
        try {
            while (true) {
                Duration timeout = Duration.ofSeconds(60);
                ReceivedMessage msg = session.getMessageAsync(timeout).get();

                String text = new String(msg.payload());
                System.out.println(Colors.instancePrefix(instanceId) + "📨 Received: " + text);

                String reply = text + " from " + instanceId;
                session.publishToAndWaitAsync(msg.context(), reply.getBytes(), null, null).get();

                System.out.println(Colors.instancePrefix(instanceId) + "📤 Replied: " + reply);
            }
        } catch (Exception e) {
            System.out.println(Colors.instancePrefix(instanceId) + "🔚 Session ended: " + e.getMessage());
        } finally {
            try {
                app.deleteSessionAndWaitAsync(session).get();
                System.out.println(Colors.instancePrefix(instanceId) + "👋 Session closed");
            } catch (Exception e) {
                System.err.println(Colors.instancePrefix(instanceId) +
                        "⚠️  Warning: failed to delete session: " + e.getMessage());
            }
        }
    }

    private static void printUsage() {
        System.out.println("\nUsage: PointToPoint --local <org/ns/app> [OPTIONS]");
        System.out.println("\nOptions:");
        System.out.println("  --local <id>              Local ID (org/namespace/app) - REQUIRED");
        System.out.println("  --remote <id>             Remote ID (org/namespace/app)");
        System.out.println("  --server <url>            SLIM server endpoint (default: http://localhost:46357)");
        System.out.println("  --message <text>          Message to send (enables sender mode)");
        System.out.println("  --iterations <n>          Number of messages to send (default: 10)");
        System.out.println("  --shared-secret <secret>  Shared secret for authentication");
        System.out.println("  --enable-mls              Enable MLS encryption");
    }
}
