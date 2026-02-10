// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples;

import io.agntcy.slim.examples.common.*;
import io.agntcy.slim.bindings.*;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.time.Duration;
import java.util.Map;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Group messaging example using Java.
 *
 * Purpose:
 * Demonstrates how to:
 * - Start a local SLIM app using the global service
 * - Optionally create a group session (becoming its moderator)
 * - Invite other participants (by their IDs) into the group
 * - Receive and display messages
 * - Interactively publish messages
 *
 * Key concepts:
 * - Group sessions are created with SessionConfig with SessionType.GROUP and
 * reference a 'topic' (channel) Name.
 * - Invites are explicit: the moderator invites each participant after
 * creating the session.
 * - Participants that did not create the session simply wait for
 * listenForSessionAsync() to yield their Session.
 *
 * Usage:
 * Moderator: --local org/ns/mod --remote org/ns/chat --invites org/ns/p1
 * --invites org/ns/p2
 * Participant: --local org/ns/p1
 */
public class Group {

    public static void main(String[] args) {
        try {
            // Parse arguments
            Config.ArgParser parser = new Config.ArgParser(args);
            Config.GroupConfig config = new Config.GroupConfig(
                    parser.getRequiredOption("local"));
            config.remote = parser.getOption("remote", null);
            config.server = parser.getOption("server", Common.DEFAULT_SERVER_ENDPOINT);
            config.sharedSecret = parser.getOption("shared-secret", Common.DEFAULT_SHARED_SECRET);
            config.enableMls = parser.hasFlag("enable-mls");
            config.invites = parser.getMultipleOptions("invites");

            // Run the client
            runClient(config);

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

    private static void runClient(Config.GroupConfig config) throws Exception {
        // Create and connect app
        Common.AppConnection connection = Common.createAndConnectApp(
                config.local, config.server, config.sharedSecret);
        App app = connection.app;
        long connId = connection.connectionId;

        long instanceId = app.id();
        System.out.println(Colors.instancePrefix(instanceId) + "✅ Created app");
        System.out.println(Colors.instancePrefix(instanceId) +
                "🔌 Connected to " + config.server + " (conn ID: " + connId + ")");

        // Session sharing between threads
        CompletableFuture<Session> sessionReady = new CompletableFuture<>();
        AtomicReference<Session> createdSessionRef = new AtomicReference<>(null);

        // Check if we're the moderator
        if (config.isModerator()) {
            // We are the moderator; create the group session now
            Name chatChannel = Common.splitId(config.remote);
            System.out.println(Colors.colored(Colors.CYAN,
                    "Creating new group session (moderator)... " + Common.splitId(config.local).toString()));

            // Create group session configuration
            SessionConfig sessionConfig = new SessionConfig(
                    SessionType.GROUP,
                    config.enableMls,
                    5, // maxRetries (Integer, not Long)
                    Duration.ofSeconds(5), // interval
                    Map.of() // metadata - use empty map instead of null
            );

            // Create session - returns a context with completion and session
            SessionWithCompletion sessionWithCompletion = app.createSession(sessionConfig, chatChannel);
            // Wait for session to be established
            sessionWithCompletion.completion().waitForCompletion();
            Session createdSession = sessionWithCompletion.session();
            createdSessionRef.set(createdSession);

            // Invite each provided participant
            for (String invite : config.invites) {
                Name inviteName = Common.splitId(invite);
                app.setRouteAsync(inviteName, connId).get();
                CompletionHandle handle = createdSession.invite(inviteName);
                handle.waitForCompletion();
                System.out.println(config.local + " -> add " + inviteName.toString() + " to the group");
            }
        }

        // Create executor for parallel tasks
        ExecutorService executor = Executors.newFixedThreadPool(2);

        // Launch receive loop
        Future<?> receiveTask = executor.submit(() -> {
            try {
                receiveLoop(app, createdSessionRef.get(), sessionReady, instanceId);
            } catch (Exception e) {
                System.err.println("Receive loop error: " + e.getMessage());
            }
        });

        // Launch keyboard loop
        Future<?> keyboardTask = executor.submit(() -> {
            try {
                keyboardLoop(app, createdSessionRef.get(), sessionReady, instanceId, config);
            } catch (Exception e) {
                System.err.println("Keyboard loop error: " + e.getMessage());
            }
        });

        // Wait for keyboard task to complete (user typed exit)
        keyboardTask.get();

        // Cancel receive task
        receiveTask.cancel(true);

        // Shutdown executor
        executor.shutdownNow();
        executor.awaitTermination(5, TimeUnit.SECONDS);

        // Clean up
        app.close();
    }

    private static void receiveLoop(App app, Session createdSession,
            CompletableFuture<Session> sessionReady, long instanceId) throws Exception {
        Session session;

        if (createdSession == null) {
            System.out.println("Waiting for session...");
            session = app.listenForSessionAsync(null).get();
        } else {
            session = createdSession;
        }

        // Signal that session is ready
        sessionReady.complete(session);

        // Get source name for display
        Name sourceName = session.source();

        while (!Thread.currentThread().isInterrupted()) {
            try {
                // Await next inbound message from the group session
                Duration timeout = Duration.ofSeconds(30);
                ReceivedMessage receivedMsg = session.getMessageAsync(timeout).get();
                MessageContext ctx = receivedMsg.context();
                byte[] payload = receivedMsg.payload();

                // Display sender name and message (using the actual sender from the context)
                String sender = ctx.sourceName().toString();
                System.out.println("\n" + sender + " > " + new String(payload));

                // If the message metadata contains PUBLISH_TO this message is a reply
                // to a previous one. In this case we do not reply to avoid loops
                if (!ctx.metadata().containsKey("PUBLISH_TO")) {
                    String reply = "message received by " + sourceName.toString();
                    session.publishToAsync(ctx, reply.getBytes(), null, ctx.metadata()).get();
                }

                // Re-print prompt
                System.out.print(sourceName.toString() + " > ");
                System.out.flush();

            } catch (InterruptedException e) {
                // Thread interrupted, exit gracefully
                break;
            } catch (ExecutionException e) {
                // Check if session is closed
                if (e.getCause() != null &&
                        e.getCause().getMessage() != null &&
                        e.getCause().getMessage().toLowerCase().contains("session closed")) {
                    break;
                }
                // Timeout, continue listening
                continue;
            } catch (Exception e) {
                // Other errors, continue
                continue;
            }
        }
    }

    private static void keyboardLoop(App app, Session createdSession,
            CompletableFuture<Session> sessionReady, long instanceId, Config.GroupConfig config)
            throws Exception {
        // Wait for the session to be established
        Session session = sessionReady.get();
        Name sourceName = session.source();
        Name destName = session.destination();

        System.out.println("\nWelcome to the group " + destName.toString() + "!");
        System.out.println("Commands:");
        System.out.println("  - Type a message to send it to the group");

        if (createdSession != null) {
            System.out.println("  - 'remove NAME' to remove a participant");
            System.out.println("  - 'invite NAME' to invite a participant");
        }

        System.out.println("  - 'exit' or 'quit' to leave the group");
        System.out.println();

        BufferedReader reader = new BufferedReader(new InputStreamReader(System.in));

        while (!Thread.currentThread().isInterrupted()) {
            System.out.print(sourceName.toString() + " > ");
            System.out.flush();

            String userInput = reader.readLine();
            if (userInput == null) {
                break;
            }

            String trimmed = userInput.trim();
            String lower = trimmed.toLowerCase();

            if (lower.equals("exit") || lower.equals("quit")) {
                if (createdSession != null) {
                    // Delete the session
                    CompletionHandle handle = app.deleteSession(session);
                    handle.waitForCompletion();
                }
                break;
            } else if (lower.startsWith("invite ") && createdSession != null) {
                String inviteId = trimmed.substring(7).trim();
                handleInvite(session, inviteId);
            } else if (lower.startsWith("remove ") && createdSession != null) {
                String removeId = trimmed.substring(7).trim();
                handleRemove(session, removeId);
            } else {
                // Send message to the channel
                session.publishAsync(userInput.getBytes(), null, null).get();
            }
        }
    }

    private static void handleInvite(Session session, String inviteId) {
        try {
            System.out.println("Inviting participant: " + inviteId);
            Name inviteName = Common.splitId(inviteId);
            CompletionHandle handle = session.invite(inviteName);
            handle.waitForCompletion();
            System.out.println("✅ Successfully invited " + inviteId);
        } catch (Exception e) {
            String errorStr = e.getMessage() != null ? e.getMessage() : "";
            if (errorStr.toLowerCase().contains("participant already in group")) {
                System.out.println("Error: Participant " + inviteId + " is already in the group.");
            } else if (errorStr.toLowerCase().contains("failed to add participant to session")) {
                System.out.println("Error: Failed to add participant " + inviteId + " to session.");
            } else {
                System.err.println("Error inviting participant: " + e.getMessage());
            }
        }
    }

    private static void handleRemove(Session session, String removeId) {
        try {
            System.out.println("Removing participant: " + removeId);
            Name removeName = Common.splitId(removeId);
            CompletionHandle handle = session.remove(removeName);
            handle.waitForCompletion();
            System.out.println("✅ Successfully removed " + removeId);
        } catch (Exception e) {
            String errorStr = e.getMessage() != null ? e.getMessage() : "";
            if (errorStr.toLowerCase().contains("participant not found in group")) {
                System.out.println("Error: Participant " + removeId + " is not in the group.");
            } else {
                System.err.println("Error removing participant: " + e.getMessage());
            }
        }
    }

    private static void printUsage() {
        System.out.println("\nUsage: Group --local <org/ns/app> [OPTIONS]");
        System.out.println("\nOptions:");
        System.out.println("  --local <id>              Local ID (org/namespace/app) - REQUIRED");
        System.out.println("  --remote <id>             Remote ID (org/namespace/channel)");
        System.out.println("  --server <url>            SLIM server endpoint (default: http://localhost:46357)");
        System.out.println("  --invites <id>            Invite participant (can be specified multiple times)");
        System.out.println("  --shared-secret <secret>  Shared secret for authentication");
        System.out.println("  --enable-mls              Enable MLS encryption");
    }
}
