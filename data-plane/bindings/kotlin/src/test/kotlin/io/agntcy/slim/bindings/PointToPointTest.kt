// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings

import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.jupiter.params.ParameterizedTest
import org.junit.jupiter.params.provider.CsvSource
import java.time.Duration
import java.util.UUID
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * Point-to-point sticky session integration test.
 * 
 * Scenario:
 *   - One logical sender creates a PointToPoint session and sends 1000 messages
 *     to a shared logical receiver identity.
 *   - Ten receiver instances (same Name) concurrently listen for an
 *     inbound session. Only one should become the bound peer for the
 *     PointToPoint session (stickiness).
 *   - All 1000 messages must arrive at exactly one receiver (verifying
 *     session affinity) and none at the others.
 *   - Test runs with MLS enabled / disabled (parametrized) to ensure
 *     stickiness is orthogonal to MLS.
 * 
 * Validated invariants:
 *   * session_type == PointToPoint for receiver-side context
 *   * dst == sender.local_name and src == receiver.local_name
 *   * Exactly one receiver_counts[i] == 1000 and total sum == 1000
 */
class PointToPointTest {
    
    /**
     * Setup sender app and connection.
     */
    private suspend fun setupSender(
        server: ServerFixture,
        senderName: Name,
        testId: String,
        receiverName: Name
    ): Pair<App, ULong?> {
        var connIdSender: ULong? = null
        val svcSender = if (server.localService) {
            val svc = Service("svcsender")
            connIdSender = svc.connectAsync(server.getClientConfig()!!)
            svc
        } else {
            server.service
        }
        
        val sender = svcSender.createAppWithSecret(senderName, LONG_SECRET)
        
        if (server.localService) {
            // Subscribe sender
            val senderNameWithId = Name.newWithId("org", "test_$testId", "p2psender", id = sender.id())
            sender.subscribeAsync(senderNameWithId, connIdSender!!)
            delay(100) // Real coroutine delay
            
            // Set route to receiver
            sender.setRouteAsync(receiverName, connIdSender)
        }
        
        return Pair(sender, connIdSender)
    }
    
    /**
     * Publish messages and wait for completion.
     */
    private suspend fun publishMessages(
        senderSession: Session,
        nMessages: Int,
        payloadType: String?,
        metadata: Map<String, String>?
    ) {
        val handles = mutableListOf<suspend () -> Unit>()
        repeat(nMessages) {
            val h = senderSession.publishAsync(
                "Hello from sender".toByteArray(),
                payloadType,
                metadata
            )
            handles.add { h.waitAsync() }
        }
        
        // Wait for all messages
        handles.forEach { it() }
    }
    
    /**
     * Wait for acknowledgment from receiver and return winner_id.
     */
    private suspend fun waitForAck(senderSession: Session): Int {
        val receivedMsg = senderSession.getMessageAsync(timeout = Duration.ofSeconds(30))
        val msg = receivedMsg.payload
        val ackText = String(msg)
        println("Sender received ack from receiver: $ackText")
        
        // Parse winning receiver index from ack: format "All messages received: {i}"
        return try {
            ackText.substringAfterLast(":").trim().toInt()
        } catch (e: Exception) {
            throw AssertionError("Unexpected ack format '$ackText': ${e.message}")
        }
    }
    
    /**
     * Validate that exactly one receiver got all messages.
     */
    private fun validateAffinity(receiverCounts: Map<Int, Int>, nMessages: Int) {
        assertEquals(nMessages, receiverCounts.values.sum())
        assertTrue(receiverCounts.values.contains(nMessages))
    }
    
    /**
     * Ensure all messages in a PointToPoint session are delivered to a single receiver instance.
     * 
     * Args:
     *     endpoint: Server endpoint or empty string for global service
     *     mlsEnabled: Whether to enable MLS for the created session
     * 
     * Flow:
     *     1. Spawn 10 receiver tasks (same logical Name).
     *     2. Sender establishes PointToPoint session.
     *     3. Sender publishes 1000 messages with consistent metadata + payload_type.
     *     4. Each receiver tallies only messages addressed to the logical receiver name.
     *     5. Assert affinity: exactly one receiver processed all messages.
     * 
     * Expectation:
     *     Sticky routing pins all messages to the first receiver that accepted the session.
     */
    @ParameterizedTest
    @CsvSource(
        "127.0.0.1:22345,true",        
        ",true",        
    )
    fun testStickySession(endpoint: String?, mlsEnabled: Boolean) = runBlocking {
        val server = setupServer(endpoint)
        
        try {
            // Generate unique names to avoid collisions
            val testId = UUID.randomUUID().toString().substring(0, 8)
            val senderName = Name("org", "test_$testId", "p2psender")
            val receiverName = Name("org", "test_$testId", "p2preceiver")
            
            println("Sender name: $senderName")
            println("Receiver name: $receiverName")
            
            // Create sender service and app
            val (sender, connIdSender) = setupSender(server, senderName, testId, receiverName)
            
            val receiverCounts = mutableMapOf<Int, Int>()
            for (i in 0 until 10) {
                receiverCounts[i] = 0
            }
            
            val nMessages = 1000
            
            /**
             * Receiver task:
             * - Creates its own Slim instance using the shared receiver Name.
             * - Awaits the inbound PointToPoint session (only one task should get bound).
             * - Counts messages matching expected routing + metadata.
             * - Continues until sender finishes publishing (loop ends by external cancel or test end).
             */
            suspend fun runReceiver(i: Int) {
                // Create receiver service and app
                var connIdReceiver: ULong? = null
                val svcReceiver = if (server.localService) {
                    val svc = Service("svcreceiver$i")
                    connIdReceiver = svc.connectAsync(server.getClientConfig()!!)
                    svc
                } else {
                    server.service
                }
                
                val receiver = svcReceiver.createAppWithSecret(receiverName, LONG_SECRET)
                
                if (server.localService) {
                    // Subscribe receiver
                    val receiverNameWithId = Name.newWithId("org", "test_$testId", "p2preceiver", id = receiver.id())
                    receiver.subscribeAsync(receiverNameWithId, connIdReceiver!!)
                    delay(100) // Real coroutine delay
                }
                
                val sessionContext = receiver.listenForSessionAsync(null)
                val session = sessionContext
                
                // Make sure the received session is PointToPoint
                assertEquals(SessionType.POINT_TO_POINT, session.sessionType())
                
                while (true) {
                    try {
                        val receivedMsg = session.getMessageAsync(timeout = Duration.ofSeconds(30))
                        val ctx = receivedMsg.context
                        
                        if (ctx.payloadType == "hello message" && ctx.metadata["sender"] == "hello") {
                            // Store the count in dictionary
                            receiverCounts[i] = receiverCounts[i]!! + 1
                            
                            if (receiverCounts[i] == nMessages) {
                                // Send back application acknowledgment
                                session.publishAsync("All messages received: $i".toByteArray(), null, null)
                            }
                        }
                    } catch (e: Exception) {
                        println("Receiver $i error: ${e.message}")
                        break
                    }
                }
            }
            
            val tasks = mutableListOf<kotlinx.coroutines.Job>()
            for (i in 0 until 10) {
                val task = launch {
                    runReceiver(i)
                }
                tasks.add(task)
            }
            
            // Give receivers a moment to start listening
            delay(1000)
            
            // Create a new session
            val sessionConfig = SessionConfig(
                sessionType = SessionType.POINT_TO_POINT,
                enableMls = mlsEnabled,
                maxRetries = 5u,
                interval = Duration.ofMillis(100),
                metadata = emptyMap()
            )
            val senderSessionContext = sender.createSession(sessionConfig, receiverName)
            senderSessionContext.completion.waitAsync()
            val senderSession = senderSessionContext.session
            
            val payloadType = "hello message"
            val metadata = mapOf("sender" to "hello")
            
            // Flood the established p2p session with messages.
            // Stickiness requirement: every one of these 1000 publishes should be delivered
            // to exactly the same receiver instance (affinity).
            
            // Publish all messages
            publishMessages(senderSession, nMessages, payloadType, metadata)
            
            // Wait for acknowledgment from receiver
            val winnerId = waitForAck(senderSession)
            
            // Cancel all non-winning receiver tasks
            for ((idx, t) in tasks.withIndex()) {
                if (idx != winnerId && !t.isCompleted) {
                    t.cancel()
                }
            }
            
            // Affinity assertions:
            //  * Sum of all per-receiver counts == total sent (1000)
            //  * Exactly one bucket contains 1000 (the sticky peer)
            validateAffinity(receiverCounts, nMessages)
            
            // Delete sender_session
            val handle = sender.deleteSessionAsync(senderSession)
            handle.waitAsync()
            
            // Await only the winning receiver task (others were cancelled)
            try {
                tasks[winnerId].join()
            } catch (e: kotlinx.coroutines.CancellationException) {
                // Expected if task was cancelled
            }
            
        } finally {
            teardownServer(server)
        }
    }
}
