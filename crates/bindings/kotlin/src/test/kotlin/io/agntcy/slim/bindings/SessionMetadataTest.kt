// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings

import kotlinx.coroutines.test.runTest
import org.junit.jupiter.api.Test
import java.time.Duration
import java.util.UUID
import kotlin.test.assertEquals
import kotlin.time.Duration.Companion.seconds

/**
 * Test: session metadata propagation (round-trip).
 * 
 * Purpose:
 *   Validate that metadata attached to a PointToPoint session configuration on the
 *   initiating side (sender) is visible with identical key/value pairs on the
 *   receiving side once the session is established.
 * 
 * What is covered:
 *   * Construction of a PointToPoint SessionConfiguration with custom metadata.
 *   * Session creation by the sender and automatic session notification for receiver.
 *   * Verification that all metadata entries appear unchanged on the receiver's
 *     session context (session_receiver.metadata).
 * 
 * Not supported:
 *   * Mutating metadata after session establishment.
 * 
 * Pass criteria:
 *   All key/value pairs inserted in the initiating configuration must appear
 *   exactly once and match on the receiver side.
 */
class SessionMetadataTest {
    
    /**
     * Ensure session metadata provided at PointToPoint session creation is preserved end-to-end.
     * 
     * Flow:
     *   1. Create sender & receiver Slim instances.
     *   2. Sender connects, sets a route to receiver.
     *   3. Sender creates a PointToPoint session with metadata.
     *   4. Sender publishes a message to trigger session establishment on receiver.
     *   5. Receiver listens for the new session and inspects metadata.
     *   6. Assert every original key/value is present and unchanged.
     * 
     * Assertions:
     *   For each (k, v) in initial metadata: receiver.metadata[k] == v.
     */
    @Test
    fun testSessionMetadataMergeRoundtrip() = runTest(timeout = 30.seconds) {
        val server = setupServer(null) // Use global service
        
        try {
            // Generate unique names to avoid collisions
            val testId = UUID.randomUUID().toString().substring(0, 8)
            val senderName = Name("org", "test_$testId", "sessionsender")
            val receiverName = Name("org", "test_$testId", "sessionreceiver")
            
            // Use global service
            val svc = server.service
            
            // Create sender and receiver apps
            val sender = svc.createAppWithSecret(senderName, LONG_SECRET)
            val receiver = svc.createAppWithSecret(receiverName, LONG_SECRET)
            
            // Metadata we want to propagate with the session creation
            val metadata = mapOf("a" to "1", "k" to "session")
            
            // Create PointToPoint session with metadata
            val sessCfg = SessionConfig(
                sessionType = SessionType.POINT_TO_POINT,
                enableMls = false,
                maxRetries = 5u,
                interval = Duration.ofSeconds(1),
                metadata = metadata
            )
            
            // Create session (returns SessionWithCompletion)
            val sessionWithCompletion = sender.createSession(sessCfg, receiverName)
            sessionWithCompletion.completion.waitAsync()
            val sessionSender = sessionWithCompletion.session
            
            // Publish a message to trigger session on receiver side
            sessionSender.publishAsync("hello".toByteArray(), null, null)
            
            // Receiver obtains the new session context
            val sessionContextReceiver = receiver.listenForSessionAsync(
                timeout = Duration.ofSeconds(10)
            )
            
            // Extract and validate metadata
            val sessionMetadata = sessionContextReceiver.metadata()
            for ((k, v) in metadata) {
                assertEquals(v, sessionMetadata[k],
                    "Metadata mismatch for key '$k': $v != ${sessionMetadata[k]}")
            }
            
            // Delete sessions
            val handle = sender.deleteSessionAsync(sessionSender)
            handle.waitAsync()
            
        } finally {
            teardownServer(server)
        }
    }
}
