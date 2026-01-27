// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples

import io.agntcy.slim.examples.common.*
import uniffi.slim_bindings.*
import kotlinx.coroutines.*
import java.time.Duration

/**
 * Point-to-point messaging example for Slim bindings.
 * 
 * This example can operate in two primary modes:
 * 
 * 1. Active sender (message mode):
 *    - Creates a session.
 *    - Publishes a fixed or user-supplied message multiple times to a remote identity.
 *    - Receives replies for each sent message (request/reply pattern).
 * 
 * 2. Passive listener (no --message provided):
 *    - Waits for inbound sessions initiated by a remote party.
 *    - Echoes replies for each received payload, tagging them with the local instance ID.
 * 
 * Key concepts demonstrated:
 *   - Global service usage with createAppWithSecret()
 *   - PointToPoint session creation logic.
 *   - Publish / receive loop with per-message reply.
 *   - Simple flow control via iteration count and sleeps (demo-friendly).
 * 
 * Notes:
 *   * PointToPoint sessions stick to one specific peer (sticky / affinity semantics).
 */

/**
 * Core coroutine that performs either active send or passive listen logic.
 * 
 * @param config PointToPointConfig instance containing all configuration
 * 
 * Behavior:
 *   - Builds Slim app using global service.
 *   - If message is supplied -> create session & publish + receive replies.
 *   - If message not supplied -> wait indefinitely for inbound sessions and echo payloads.
 */
suspend fun runClient(config: PointToPointConfig) = coroutineScope {
    // Build the Slim application using global service
    val (localApp, connId) = createLocalApp(config)
    
    // Numeric unique instance ID (useful for distinguishing multiple processes)
    val instance = localApp.id().toString()
    
    // If user intends to send messages, remote must be provided for routing
    if (config.message != null && config.remote == null) {
        throw IllegalArgumentException("Remote ID must be provided when message is specified.")
    }
    
    // ACTIVE MODE (publishing + expecting replies)
    if (config.message != null && config.remote != null) {
        // Convert the remote ID string into a Name
        val remoteName = splitId(config.remote)
        
        // Create local route to enable forwarding towards remote name
        localApp.setRouteAsync(remoteName, connId)
        
        // Create point-to-point session configuration
        val sessionConfig = SessionConfig(
            sessionType = SessionType.POINT_TO_POINT,
            enableMls = config.enableMls,
            maxRetries = 5u,
            interval = Duration.ofSeconds(5),
            metadata = emptyMap()
        )
        
        // Create session - returns a context with completion and session
        val sessionContext = localApp.createSessionAsync(sessionConfig, remoteName)
        // Wait for session to be established
        sessionContext.completion.waitAsync()
        val session = sessionContext.session
        
        val sessionId = session.sessionId()
        
        var sessionClosed = false
        // Iterate send->receive cycles
        repeat(config.iterations) { i ->
            try {
                // Publish message to the session
                session.publishAsync(config.message.toByteArray(), null, null)
                Colors.printFormatted(
                    instance,
                    "Sent message ${config.message} - ${i + 1}/${config.iterations}"
                )
                
                // Wait for reply from remote peer
                val receivedMsg = session.getMessageAsync(Duration.ofSeconds(30))
                val reply = String(receivedMsg.payload)
                Colors.printFormatted(
                    instance,
                    "received (from session $sessionId): $reply"
                )
            } catch (e: Exception) {
                // Surface an error but continue attempts
                Colors.printFormatted(instance, "error: ${e.message}")
                // if the session is closed exit
                if (e.message?.contains("session closed", ignoreCase = true) == true) {
                    sessionClosed = true
                    return@repeat
                }
            }
            // 1s pacing so output remains readable
            delay(1000)
        }
        
        if (!sessionClosed) {
            // Delete session
            val handle = localApp.deleteSessionAsync(session)
            handle.waitAsync()
        }
    }
    // PASSIVE MODE (listen for inbound sessions)
    else {
        while (isActive) {
            Colors.printFormatted(instance, "waiting for new session to be established")
            
            // Block until a remote peer initiates a session to us
            val session = localApp.listenForSessionAsync(null)
            val sessionId = session.sessionId()
            Colors.printFormatted(instance, "new session $sessionId")
            
            // Launch a dedicated task to handle this session (allow multiple)
            launch {
                sessionLoop(session, instance)
            }
        }
    }
}

/**
 * Inner loop for a single inbound session:
 *   * Receive messages until the session is closed or an error occurs.
 *   * Echo each message back using publish.
 */
suspend fun sessionLoop(session: Session, instance: String) {
    while (true) {
        try {
            val receivedMsg = session.getMessageAsync(Duration.ofSeconds(30))
            val payload = receivedMsg.payload
            val text = String(payload)
            Colors.printFormatted(instance, "received: $text")
            
            // Echo reply with appended instance identifier
            session.publishAsync("$text from $instance".toByteArray(), null, null)
        } catch (e: Exception) {
            // Session likely closed or transport broken
            break
        }
    }
}

/**
 * Main entry point for point-to-point example.
 * 
 * Parses command-line arguments and runs the client.
 */
fun main(args: Array<String>) = runBlocking {
    try {
        val config = ConfigParser.parsePointToPointArgs(args)
        
        // Run the client
        runClient(config)
    } catch (e: IllegalArgumentException) {
        println("Configuration error: ${e.message}")
        println()
        println("Usage: PointToPoint --local <org/ns/app> [OPTIONS]")
        println()
        println("Options:")
        println("  --local <id>              Local ID (org/namespace/app) - REQUIRED")
        println("  --remote <id>             Remote ID (org/namespace/app)")
        println("  --slim <url>              SLIM server endpoint (default: http://127.0.0.1:46357)")
        println("  --message <text>          Message to send (activates sender mode)")
        println("  --iterations <n>          Number of request/reply cycles (default: 10)")
        println("  --shared-secret <secret>  Shared secret for authentication")
        println("  --enable-mls              Enable MLS encryption")
        println("  --enable-opentelemetry    Enable OpenTelemetry tracing")
        kotlin.system.exitProcess(1)
    } catch (e: CancellationException) {
        println("\nClient interrupted by user.")
    } catch (e: Exception) {
        println("Error: ${e.message}")
        e.printStackTrace()
        kotlin.system.exitProcess(1)
    }
}
