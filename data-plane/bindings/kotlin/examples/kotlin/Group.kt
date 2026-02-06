// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples

import io.agntcy.slim.examples.common.*
import io.agntcy.slim.bindings.*
import kotlinx.coroutines.*
import java.time.Duration
import java.util.concurrent.atomic.AtomicReference

/**
 * Group example (heavily commented).
 * 
 * Purpose:
 *   Demonstrates how to:
 *     * Start a local Slim app using the global service
 *     * Optionally create a group session (becoming its moderator)
 *     * Invite other participants (by their IDs) into the group
 *     * Receive and display messages
 *     * Interactively publish messages
 * 
 * Key concepts:
 *   - Group sessions are created with SessionConfig with SessionType.GROUP and
 *     reference a 'topic' (channel) Name.
 *   - Invites are explicit: the moderator invites each participant after
 *     creating the session.
 *   - Participants that did not create the session simply wait for
 *     listenForSessionAsync() to yield their Session.
 * 
 * Usage:
 *   Group --local org/default/me --remote org/default/chat-topic \
 *         --invites org/default/peer1 --invites org/default/peer2
 * 
 * Notes:
 *   * If --invites is omitted, the client runs in passive participant mode.
 *   * If both remote and invites are supplied, the client acts as session moderator.
 */

/**
 * Handle inviting a participant to the group.
 */
suspend fun handleInvite(session: Session, inviteId: String) {
    val parts = inviteId.trim().split(" ")
    if (parts.size != 1) {
        println("Error: 'invite' command expects exactly one participant ID (e.g., 'invite org/ns/client-1')")
        return
    }
    
    println("Inviting participant: $inviteId")
    val inviteName = splitId(inviteId)
    try {
        val handle = session.inviteAsync(inviteName)
        handle.waitAsync()
        println("✅ Successfully invited $inviteId")
    } catch (e: Exception) {
        val errorStr = e.message ?: ""
        when {
            errorStr.contains("participant already in group", ignoreCase = true) -> {
                println("Error: Participant $inviteId is already in the group.")
            }
            errorStr.contains("failed to add participant to session", ignoreCase = true) -> {
                println("Error: Failed to add participant $inviteId to session.")
            }
            else -> throw e
        }
    }
}

/**
 * Handle removing a participant from the group.
 */
suspend fun handleRemove(session: Session, removeId: String) {
    val parts = removeId.trim().split(" ")
    if (parts.size != 1) {
        println("Error: 'remove' command expects exactly one participant ID (e.g., 'remove org/ns/client-1')")
        return
    }
    
    println("Removing participant: $removeId")
    val removeName = splitId(removeId)
    try {
        val handle = session.removeAsync(removeName)
        handle.waitAsync()
        println("✅ Successfully removed $removeId")
    } catch (e: Exception) {
        val errorStr = e.message ?: ""
        when {
            errorStr.contains("participant not found in group", ignoreCase = true) -> {
                println("Error: Participant $removeId is not in the group.")
            }
            else -> throw e
        }
    }
}

/**
 * Receive messages for the bound session.
 * 
 * Behavior:
 *   * If not moderator: wait for a new group session (listenForSessionAsync()).
 *   * If moderator: reuse the createdSession reference.
 *   * Loop forever until cancellation or an error occurs.
 */
suspend fun receiveLoop(
    localApp: App,
    createdSession: Session?,
    sessionReady: CompletableDeferred<Session>
) {
    val session = createdSession ?: run {
        println("Waiting for session...")
        localApp.listenForSessionAsync(null)
    }
    
    // Signal that session is ready
    sessionReady.complete(session)
    
    // Get source name for display
    val sourceName = session.source()
    
    while (true) {
        try {
            // Await next inbound message from the group session
            val receivedMsg = session.getMessageAsync(Duration.ofSeconds(30))
            val ctx = receivedMsg.context
            val payload = receivedMsg.payload
            
            // Display sender name and message (using the actual sender from the context)
            val sender = ctx.sourceName.toIdString()
            println("$sender > ${String(payload)}")
            
            // if the message metadata contains PUBLISH_TO this message is a reply
            // to a previous one. In this case we do not reply to avoid loops
            if (!ctx.metadata.containsKey("PUBLISH_TO")) {
                val reply = "message received by ${sourceName.toIdString()}"
                session.publishToAsync(ctx, reply.toByteArray(), null, ctx.metadata)
            }
        } catch (e: CancellationException) {
            // Graceful shutdown path (ctrl-c or program exit)
            break
        } catch (e: Exception) {
            // Break if session is closed, otherwise continue listening
            if (e.message?.contains("session closed", ignoreCase = true) == true) {
                break
            }
            continue
        }
    }
}

/**
 * Interactive loop allowing participants to publish messages.
 * 
 * Typing 'exit' or 'quit' (case-insensitive) terminates the loop.
 * Typing 'remove NAME' removes a participant from the group
 * Typing 'invite NAME' invites a participant to the group
 * Each line is published to the group channel as UTF-8 bytes.
 */
suspend fun keyboardLoop(
    createdSession: Session?,
    sessionReady: CompletableDeferred<Session>,
    localApp: App
) {
    try {
        // Wait for the session to be established
        val session = sessionReady.await()
        val sourceName = session.source()
        val destName = session.destination()
        
        if (createdSession != null) {
            println("Welcome to the group ${destName.toIdString()}!")
            println("Commands:")
            println("  - Type a message to send it to the group")
            println("  - 'remove NAME' to remove a participant")
            println("  - 'invite NAME' to invite a participant")
            println("  - 'exit' or 'quit' to leave the group")
        } else {
            println("Welcome to the group ${destName.toIdString()}!")
            println("Commands:")
            println("  - Type a message to send it to the group")
            println("  - 'exit' or 'quit' to leave the group")
        }
        
        while (true) {
            val userInput = readlnOrNull() ?: break
            
            when {
                userInput.trim().lowercase() in listOf("exit", "quit") -> {
                    if (createdSession != null) {
                        // Delete the session
                        val handle = localApp.deleteSessionAsync(session)
                        handle.waitAsync()
                    }
                    break
                }
                
                userInput.trim().lowercase().startsWith("invite ") && createdSession != null -> {
                    val inviteId = userInput.substring(7).trim()
                    handleInvite(session, inviteId)
                    continue
                }
                
                userInput.trim().lowercase().startsWith("remove ") && createdSession != null -> {
                    val removeId = userInput.substring(7).trim()
                    handleRemove(session, removeId)
                    continue
                }
                
                else -> {
                    // Send message to the channel_name specified when creating the session.
                    // As the session is group, all participants will receive it.
                    session.publishAsync(userInput.toByteArray(), null, null)
                }
            }
        }
    } catch (e: CancellationException) {
        // Handle task cancellation gracefully
    } catch (e: Exception) {
        println("-> Error sending message: ${e.message}")
    }
}

/**
 * Orchestrate one group-capable client instance.
 * 
 * Modes:
 *   * Moderator (creator): remote (channel) + invites provided.
 *   * Listener only: no remote; waits for inbound group sessions.
 * 
 * @param config GroupConfig instance containing all configuration
 */
suspend fun runClient(config: GroupConfig) = coroutineScope {
    // Create the local Slim instance using global service
    val (localApp, connId) = createLocalApp(config)
    
    // Parse the remote channel/topic if provided; else None triggers passive mode
    val chatChannel = config.remote?.let { splitId(it) }
    
    // Session sharing between tasks
    val sessionReady = CompletableDeferred<Session>()
    
    // Session object only exists immediately if we are moderator
    var createdSession: Session? = null
    if (chatChannel != null && config.invites != null) {
        // We are the moderator; create the group session now
        Colors.printFormatted(
            "Creating new group session (moderator)... ${splitId(config.local).toIdString()}"
        )
        
        // Create group session configuration
        val sessionConfig = SessionConfig(
            sessionType = SessionType.GROUP,
            enableMls = config.enableMls,
            maxRetries = 5u,
            interval = Duration.ofSeconds(5),
            metadata = emptyMap()
        )
        
        // Create session - returns a context with completion and session
        val sessionContext = localApp.createSession(sessionConfig, chatChannel)
        // Wait for session to be established
        sessionContext.completion.waitAsync()
        createdSession = sessionContext.session
        
        // Invite each provided participant
        config.invites.forEach { invite ->
            val inviteName = splitId(invite)
            localApp.setRouteAsync(inviteName, connId)
            val handle = createdSession.inviteAsync(inviteName)
            handle.waitAsync()
            println("${config.local} -> add ${inviteName.toIdString()} to the group")
        }
    }
    
    // Launch the receiver and keyboard loop on separate threads (parallel execution)
    val receiveJob = launch(Dispatchers.IO) {
        receiveLoop(localApp, createdSession, sessionReady)
    }
    
    val keyboardJob = launch(Dispatchers.IO) {
        keyboardLoop(createdSession, sessionReady, localApp)
    }
    
    // Wait for keyboard loop to finish (user typed exit)
    keyboardJob.join()
    
    // Cancel the receive loop
    receiveJob.cancel()
}

/**
 * Main entry point for the group example.
 * 
 * Parses command-line arguments and runs the client.
 */
fun main(args: Array<String>) = runBlocking {
    try {
        val config = ConfigParser.parseGroupArgs(args)
        
        // Run the client
        runClient(config)
    } catch (e: IllegalArgumentException) {
        println("Configuration error: ${e.message}")
        println()
        println("Usage: Group --local <org/ns/app> [OPTIONS]")
        println()
        println("Options:")
        println("  --local <id>              Local ID (org/namespace/app) - REQUIRED")
        println("  --remote <id>             Remote ID (org/namespace/channel)")
        println("  --slim <url>              SLIM server endpoint (default: http://127.0.0.1:46357)")
        println("  --invites <id>            Invite participant (can be specified multiple times)")
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
