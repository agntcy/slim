// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples

import io.agntcy.slim.examples.common.*
import uniffi.slim_bindings.*
import kotlinx.coroutines.*

/**
 * Slim server example (extensively commented).
 * 
 * This module demonstrates:
 *   * Initializing tracing (optionally enabling OpenTelemetry export)
 *   * Spinning up a Slim service in server mode using the global service
 *   * Graceful shutdown via SIGINT (Ctrl+C)
 * 
 * High-level flow:
 *   main() -> runBlocking { runServer() }
 *       runServer():
 *         * Parse CLI flags (address, OTEL toggle)
 *         * Initialize global state and tracing
 *         * Start the Slim server (managed by Rust runtime)
 *         * Register SIGINT handler for graceful shutdown
 *         * Wait until Ctrl+C is pressed
 *         * Stop the server
 * 
 * Tracing:
 *   When --enable-opentelemetry is passed, OTEL export is enabled towards
 *   localhost:4317 (default OTLP gRPC collector). If no collector is running,
 *   tracing initialization will still succeed but spans may be dropped.
 */

/**
 * Async entry point for server.
 * 
 * Steps:
 *     1. Initialize tracing and global service.
 *     2. Start the server (Rust manages the server lifecycle).
 *     3. Register shutdown handler and wait for shutdown signal.
 *     4. Stop the server gracefully.
 * 
 * @param config ServerConfig instance containing all configuration
 */
suspend fun runServer(config: ServerConfig) = coroutineScope {
    // Get the global service instance
    val service = setupService(config.enableOpentelemetry)
    
    // Launch the embedded server with insecure TLS (development setting).
    // The server runs in the Rust runtime and is managed there.
    val serverConfig = newInsecureServerConfig(config.slim)
    service.runServerAsync(serverConfig)
    
    println("Slim server started on ${config.slim}")
    println("Press Ctrl+C to stop the server")
    
    // Register shutdown hook
    Runtime.getRuntime().addShutdownHook(Thread {
        println("\nShutting down server...")
        runBlocking {
            try {
                service.shutdownAsync()
                println("Server stopped at ${config.slim}")
            } catch (e: Exception) {
                println("Error stopping server: ${e.message}")
            }
        }
    })
    
    // Block until cancellation
    try {
        awaitCancellation()
    } catch (e: CancellationException) {
        // Expected when shutdown is triggered
        println("Server shutdown initiated")
    }
}

/**
 * Main entry point for the server example.
 * 
 * Parses command-line arguments and runs the server.
 */
fun main(args: Array<String>) = runBlocking {
    try {
        val config = ConfigParser.parseServerArgs(args)
        
        // Run the server
        runServer(config)
    } catch (e: IllegalArgumentException) {
        println("Configuration error: ${e.message}")
        println()
        println("Usage: Server [OPTIONS]")
        println()
        println("Options:")
        println("  --slim, -s <address>      SLIM server address (host:port) (default: 127.0.0.1:46357)")
        println("  --enable-opentelemetry, -t  Enable OpenTelemetry tracing")
        kotlin.system.exitProcess(1)
    } catch (e: CancellationException) {
        println("\nServer terminated by user.")
    } catch (e: Exception) {
        println("Error: ${e.message}")
        e.printStackTrace()
        kotlin.system.exitProcess(1)
    }
}
