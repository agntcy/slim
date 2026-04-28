// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples;

import io.agntcy.slim.examples.common.*;
import io.agntcy.slim.bindings.*;

import java.util.concurrent.CountDownLatch;

/**
 * SLIM server example using Java.
 *
 * This demonstrates:
 * - Initializing tracing (optionally enabling OpenTelemetry export)
 * - Starting a SLIM service in server mode using the global service
 * - Graceful shutdown via shutdown hook (Ctrl+C)
 *
 * High-level flow:
 * 1. Parse CLI flags (address, OTEL toggle)
 * 2. Initialize global state and tracing
 * 3. Start the SLIM server (managed by Rust runtime)
 * 4. Register shutdown handler for graceful shutdown
 * 5. Wait until Ctrl+C is pressed
 * 6. Stop the server
 *
 * Tracing:
 * When --enable-opentelemetry is passed, OTEL export is enabled towards
 * localhost:4317 (default OTLP gRPC collector). If no collector is running,
 * tracing initialization will still succeed but spans may be dropped.
 */
public class Server {

    public static void main(String[] args) {
        try {
            // Parse arguments
            Config.ArgParser parser = new Config.ArgParser(args);
            Config.ServerConfig config = new Config.ServerConfig();
            config.configPath = parser.getOption("config", null);
            config.enableOpentelemetry = parser.hasFlag("enable-opentelemetry");

            String slimAddress = parser.getOption("slim", "127.0.0.1:46357");

            // Run the server
            runServer(slimAddress, config);

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

    private static void runServer(String slimAddress, Config.ServerConfig config) throws Exception {
        // Initialize crypto, runtime, global service and logging
        if (config.enableOpentelemetry) {
            System.out.println("🔍 Initializing with OpenTelemetry tracing...");
            // TODO: Configure OpenTelemetry through config files or environment variables
            // For now, use default initialization
            SlimBindings.initializeWithDefaults();
        } else {
            SlimBindings.initializeWithDefaults();
        }

        Service service = SlimBindings.getGlobalService();

        // Create server configuration
        ServerConfig serverConfig = SlimBindings.newInsecureServerConfig(slimAddress);

        // Start the server (async)
        service.runServerAsync(serverConfig).get();

        System.out.println("🚀 SLIM server started on " + slimAddress);
        System.out.println("Press Ctrl+C to stop the server");

        // Latch to keep main thread alive
        CountDownLatch shutdownLatch = new CountDownLatch(1);

        // Register shutdown hook for graceful termination
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            System.out.println("\n🛑 Shutting down server...");
            try {
                service.shutdownAsync().get();
                System.out.println("✅ Server stopped at " + slimAddress);
            } catch (Exception e) {
                System.err.println("⚠️  Error stopping server: " + e.getMessage());
            } finally {
                shutdownLatch.countDown();
            }
        }));

        // Wait for shutdown signal
        try {
            shutdownLatch.await();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            System.out.println("Server interrupted");
        }
    }

    private static void printUsage() {
        System.out.println("\nUsage: Server [OPTIONS]");
        System.out.println("\nOptions:");
        System.out.println("  --slim <address>           SLIM server address (host:port) (default: 127.0.0.1:46357)");
        System.out.println("  --config <path>            Configuration file path (optional)");
        System.out.println("  --enable-opentelemetry     Enable OpenTelemetry tracing");
    }
}
