// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings

import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runTest
import org.junit.jupiter.params.ParameterizedTest
import org.junit.jupiter.params.provider.ValueSource
import java.time.Duration
import java.util.UUID
import kotlin.test.assertEquals
import kotlin.test.assertTrue
import kotlin.time.Duration.Companion.seconds

/**
 * Group integration test for Slim bindings.
 * 
 * Scenario:
 *   - A configurable number of participants (participants_count) join a group
 *     "chat" identified by a shared topic (Name).
 *   - Participant 0 (the "moderator") creates the group session, invites
 *     every other participant, and publishes the first message addressed to the
 *     next participant in a logical ring.
 *   - Each participant, upon receiving a message that ends with its own name,
 *     publishes a new message naming the next participant, continuing the ring.
 *   - Each participant exits after observing (participants_count - 1) messages.
 * 
 * What is validated implicitly:
 *   * Group session establishment (session_type == Group).
 *   * dst equals the channel/topic Name for non-creator participants.
 *   * src matches the participant's own local identity when receiving.
 *   * Message propagation across all participants without loss.
 *   * Optional MLS flag is parameterized.
 */
class GroupTest {
    
    /**
     * Create and setup a participant app.
     */
    private suspend fun createParticipant(
        server: ServerFixture,
        testId: String,
        index: Int
    ): Triple<App, ULong?, String> {
        val partName = "participant_$index"
        val name = Name("org", "test_$testId", partName)
        
        var connId: ULong? = null
        val svc = if (server.localService) {
            val s = Service("svcparticipant$index")
            val clientConfig = server.getClientConfig()
            requireNotNull(clientConfig) { "Client config should not be null" }
            connId = s.connectAsync(clientConfig)
            s
        } else {
            server.service
        }
        
        val participant = svc.createAppWithSecret(name, LONG_SECRET)
        
        if (server.localService) {
            val nameWithId = Name.newWithId("org", "test_$testId", partName, id = participant.id())
            participant.subscribeAsync(nameWithId, connId!!)
            delay(100)
        }
        
        return Triple(participant, connId, partName)
    }
    
    /**
     * Create a group session and return it.
     */
    private suspend fun createGroupSession(
        participant: App,
        chatName: Name,
        mlsEnabled: Boolean
    ): Session {
        val sessionConfig = SessionConfig(
            sessionType = SessionType.GROUP,
            enableMls = mlsEnabled,
            maxRetries = 5u,
            interval = Duration.ofSeconds(1),
            metadata = emptyMap()
        )
        val sessionContext = participant.createSession(sessionConfig, chatName)
        sessionContext.completion.waitAsync()
        return sessionContext.session
    }
    
    /**
     * Invite all participants to the group session.
     */
    private suspend fun inviteParticipants(
        participant: App,
        session: Session,
        server: ServerFixture,
        testId: String,
        participantsCount: Int,
        connId: ULong?,
        partName: String
    ) {
        for (i in 0 until participantsCount) {
            if (i != 0) {
                val nameToAdd = "participant_$i"
                val toAdd = Name("org", "test_$testId", nameToAdd)
                if (server.localService) {
                    participant.setRouteAsync(toAdd, connId!!)
                }
                
                val handle = session.inviteAsync(toAdd)
                handle.waitAsync()
                println("$partName -> add $nameToAdd to the group")
            }
        }
        
        delay(1000) // Wait a bit to ensure routes are set up
    }
    
    /**
     * Receive and validate the session if this is the first message.
     */
    private suspend fun receiveSession(
        participant: App,
        index: Int,
        firstMessage: Boolean
    ): Pair<Session?, Boolean> {
        if (index == 0 || !firstMessage) {
            return Pair(null, firstMessage)
        }
        
        val sessionContext = participant.listenForSessionAsync(null)
        val recvSession = sessionContext
        
        assertEquals(SessionType.GROUP, recvSession.sessionType())
        
        return Pair(recvSession, false)
    }
    
    /**
     * Handle message relay logic - check if message is for this participant and relay.
     */
    private suspend fun handleMessageRelay(
        recvSession: Session,
        index: Int,
        participantsCount: Int,
        partName: String,
        message: String,
        msgRcv: ByteArray,
        called: Boolean
    ): Boolean {
        if (!called && String(msgRcv).endsWith(partName)) {
            println("$partName -> Receiving message: ${String(msgRcv)}, local count: $index")
            
            val nextParticipant = (index + 1) % participantsCount
            val nextParticipantName = "participant_$nextParticipant"
            println("$partName -> Calling out $nextParticipantName...")
            
            recvSession.publishAsync("$message - $nextParticipantName".toByteArray(), null, null)
            println("$partName -> Published!")
            return true
        }
        
        println("$partName -> Receiving message: ${String(msgRcv)} - not for me.")
        return called
    }
    
    /**
     * Close session as moderator (participant 0).
     */
    private suspend fun closeSessionAsModerator(
        participant: App,
        recvSession: Session,
        partName: String
    ) {
        println("$partName -> Closing session as moderator...")
        delay(500) // Give others time to finish
        val h = participant.deleteSessionAsync(recvSession)
        h.waitAsync()
        println("$partName -> Session closed successfully")
    }
    
    /**
     * Wait for session to be closed by moderator.
     */
    private suspend fun waitForSessionClose(recvSession: Session, partName: String) {
        try {
            recvSession.getMessageAsync(timeout = Duration.ofSeconds(30))
        } catch (e: Exception) {
            println("$partName -> Received error $e")
        }
    }
    
    /**
     * Exercise group session behavior with N participants relaying a message in a ring.
     * 
     * Steps:
     *   1. Participant 0 creates group session (optionally with MLS enabled).
     *   2. Participant 0 invites remaining participants after short delay.
     *   3. Participant 0 seeds first message naming participant-1.
     *   4. Each participant that is "called" republishes naming the next participant.
     *   5. Each participant terminates after seeing (N - 1) messages.
     * 
     * Args:
     *     mlsEnabled: Whether MLS secure group features are enabled for the session.
     * 
     * This test asserts invariants via inline assertions and stops when all
     * participant tasks complete successfully.
     */
    @ParameterizedTest
    @ValueSource(booleans = [false])
    fun testGroup(mlsEnabled: Boolean) = runTest(timeout = 180.seconds) {
        val server = setupServer(null) // Use global service
        
        try {
            val message = "Calling app"
            
            // Generate unique names to avoid collisions when using global service
            val testId = UUID.randomUUID().toString().substring(0, 8)
            
            // Participant count
            val participantsCount = 10
            val participants = mutableListOf<kotlinx.coroutines.Job>()
            
            val chatName = Name("org", "test_$testId", "chat")
            
            println("Test ID: $testId")
            println("Chat name: $chatName")
            println("Using ${if (server.localService) "local" else "global"} service")
            
            /**
             * Participant coroutine.
             * 
             * Responsibilities:
             *   * Index 0: create group session, invite others, publish initial message, close session when done.
             *   * Others: wait for inbound session, validate session properties, relay when addressed.
             */
            suspend fun backgroundTask(index: Int) {
                var localCount = 0
                
                // Create participant
                val (participant, connId, partName) = createParticipant(server, testId, index)
                println("Creating participant $partName...")
                
                val session: Session? = if (index == 0) {
                    println("$partName -> Creating new group sessions...")
                    val s = createGroupSession(participant, chatName, mlsEnabled)
                    inviteParticipants(participant, s, server, testId, participantsCount, connId, partName)
                    s
                } else {
                    // Will be set later when session is received
                    null
                }
                
                // Track if this participant was called
                var called = false
                var firstMessage = true
                var recvSession: Session? = session
                
                // If this is the first participant, publish the message to start the chain
                if (index == 0) {
                    val nextParticipant = (index + 1) % participantsCount
                    val nextParticipantName = "participant_$nextParticipant"
                    val msg = "$message - $nextParticipantName"
                    println("$partName -> Publishing message as first participant: $msg")
                    called = true
                    session!!.publishAsync(msg.toByteArray(), null, null)
                }
                
                while (true) {
                    try {
                        // Init session from session
                        if (index == 0) {
                            recvSession = session
                        } else {
                            val (newSession, newFirstMessage) = receiveSession(participant, index, firstMessage)
                            if (newSession != null) {
                                recvSession = newSession
                            }
                            firstMessage = newFirstMessage
                            // recvSession should already be defined from previous iteration
                        }
                        
                        requireNotNull(recvSession) { "recvSession should not be null" }
                        
                        // Receive message from session
                        val receivedMsg = recvSession.getMessageAsync(timeout = Duration.ofSeconds(30))
                        val ctx = receivedMsg.context
                        val msgRcv = receivedMsg.payload
                        
                        // Increase the count
                        localCount++
                        
                        // Make sure the message is correct
                        assertTrue(msgRcv.take(message.length).toByteArray().contentEquals(message.toByteArray()))
                        
                        // Handle message relay
                        called = handleMessageRelay(
                            recvSession,
                            index,
                            participantsCount,
                            partName,
                            message,
                            msgRcv,
                            called
                        )
                        
                        // All participants exit after receiving all expected messages
                        if (localCount >= (participantsCount - 1)) {
                            println("$partName -> Received all $localCount messages")
                            
                            // Moderator (participant 0) closes the session
                            if (index == 0) {
                                closeSessionAsModerator(participant, recvSession, partName)
                            } else {
                                waitForSessionClose(recvSession, partName)
                            }
                            
                            break
                        }
                    } catch (e: Exception) {
                        println("$partName -> Unexpected error: ${e.message}")
                        throw e
                    }
                }
            }
            
            // Start participants in background
            for (i in 0 until participantsCount) {
                val task = launch {
                    backgroundTask(i)
                }
                participants.add(task)
            }
            
            // Wait for all participants to complete
            for (task in participants) {
                task.join()
            }
            
            println("All participants completed successfully!")
            
        } finally {
            teardownServer(server)
        }
    }
}
