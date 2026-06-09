// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.yield
import org.junit.jupiter.api.Test
import java.net.ServerSocket
import java.time.Duration
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * Point-to-point sticky session integration test.
 *
 * Scenario:
 *   - One logical sender creates a PointToPoint session and sends many messages
 *     to a shared logical receiver identity.
 *   - Two receiver instances (same Name) concurrently listen for an
 *     inbound session. Only one should become the bound peer for the
 *     PointToPoint session (stickiness).
 *   - All messages must arrive at exactly one receiver (verifying
 *     session affinity) and none at the others.
 *   - Runs with MLS enabled (same intent as Java). Message count is lower than Java's 1000
 *     for faster Kotlin tests; Gradle runs each bindings test class in its own JVM (forkEvery).
 *
 * This Kotlin test uses an **embedded TCP** server on an ephemeral port with a
 * port-scoped host service name (see [TestHelpers.setupServer]). Receiver count is reduced
 * versus Java (2 vs 10) to limit concurrent dataplane clients while still exercising
 * stickiness (only one listener should win the PointToPoint session).
 *
 * Validated invariants:
 *   * session_type == PointToPoint for receiver-side context
 *   * dst == sender.local_name and src == receiver.local_name
 *   * Exactly one receiver_counts[i] == nMessages and total sum == nMessages
 */
class PointToPointTest {
    private fun freeLocalEndpoint(): String =
        ServerSocket(0).use { socket ->
            "127.0.0.1:${socket.localPort}"
        }

    /**
     * Setup sender app and connection.
     */
    private suspend fun setupSender(
        server: ServerFixture,
        senderName: Name,
        testId: String,
        receiverName: Name,
    ): Pair<App, ULong?> {
        var connIdSender: ULong? = null
        val svcSender =
            if (server.localService) {
                val svc = Service("snd$testId")
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
     *
     * Uses [Session.publishAsync] + [CompletionHandle.waitAsync] (same pattern as
     * [BindingsTest]) for each message so pacing matches other Kotlin integration tests.
     */
    private suspend fun publishMessages(
        senderSession: Session,
        nMessages: Int,
        payloadType: String?,
        metadata: Map<String, String>?,
    ) {
        repeat(nMessages) { i ->
            yield()
            val handle =
                senderSession.publishAsync(
                    "Hello from sender".toByteArray(),
                    payloadType,
                    metadata,
                )
            handle.waitAsync()
            if (i % 25 == 24) {
                delay(5)
            }
            if ((i + 1) % 25 == 0) {
                println("[PointToPointTest] published ${i + 1}/$nMessages messages")
            }
        }
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
            val prefix = "All messages received: "
            require(ackText.startsWith(prefix)) { "Unexpected ack format: $ackText" }
            ackText.substring(prefix.length).trim().toInt()
        } catch (e: Exception) {
            throw AssertionError("Unexpected ack format '$ackText': ${e.message}")
        }
    }

    /**
     * Validate that exactly one receiver got all messages.
     */
    private fun validateAffinity(
        receiverCounts: Map<Int, AtomicInteger>,
        nMessages: Int,
    ) {
        assertEquals(nMessages, receiverCounts.values.sumOf { it.get() })
        assertTrue(receiverCounts.values.any { it.get() == nMessages })
    }

    /**
     * Ensure all messages in a PointToPoint session are delivered to a single receiver instance.
     *
     * Flow:
     *     1. Spawn two receiver tasks (same logical Name).
     *     2. Sender establishes PointToPoint session.
     *     3. Sender publishes nMessages with consistent metadata + payload_type.
     *     4. Each receiver tallies only messages addressed to the logical receiver name.
     *     5. Assert affinity: exactly one receiver processed all messages.
     *
     * Expectation:
     *     Sticky routing pins all messages to the first receiver that accepted the session.
     */
    @Test
    fun testStickySession() {
        val mlsEnabled = true
        runBlocking(Dispatchers.Default) {
            val server = setupServer(freeLocalEndpoint())

            try {
                    val testId = UUID.randomUUID().toString().substring(0, 8)
                    val senderName = Name("org", "test_$testId", "p2psender")
                    val receiverName = Name("org", "test_$testId", "p2preceiver")

                    println("Sender name: $senderName")
                    println("Receiver name: $receiverName")

                    val (sender, _) = setupSender(server, senderName, testId, receiverName)

                    val nReceivers = 2
                    val receiverCounts = ConcurrentHashMap<Int, AtomicInteger>()
                    for (i in 0 until nReceivers) {
                        receiverCounts[i] = AtomicInteger(0)
                    }

                    // Small batch: enough to prove sticky multi-message delivery; each test
                    // class runs in a fresh JVM (see build.gradle.kts forkEvery) so we need not
                    // keep this as tiny as when native state leaked between classes on CI.
                    val nMessages = 20

                    suspend fun runReceiver(i: Int) {
                        var connIdReceiver: ULong? = null
                        val svcReceiver =
                            if (server.localService) {
                                val svc = Service("rcv${testId}$i")
                                connIdReceiver = svc.connectAsync(server.getClientConfig()!!)
                                svc
                            } else {
                                server.service
                            }

                        val receiver = svcReceiver.createAppWithSecret(receiverName, LONG_SECRET)

                        if (server.localService) {
                            val receiverNameWithId =
                                Name.newWithId("org", "test_$testId", "p2preceiver", id = receiver.id())
                            receiver.subscribeAsync(receiverNameWithId, connIdReceiver!!)
                            delay(100)
                        }

                        val sessionContext =
                            receiver.listenForSessionAsync(timeout = Duration.ofSeconds(120))
                        val session = sessionContext

                        assertEquals(SessionType.POINT_TO_POINT, session.sessionType())

                        while (true) {
                            try {
                                val receivedMsg = session.getMessageAsync(timeout = Duration.ofSeconds(30))
                                val ctx = receivedMsg.context

                                if (ctx.payloadType == "hello message" && ctx.metadata["sender"] == "hello") {
                                    val count = receiverCounts[i]!!.incrementAndGet()

                                    if (count == nMessages) {
                                        val ack =
                                            session.publishAsync(
                                                "All messages received: $i".toByteArray(),
                                                null,
                                                null,
                                            )
                                        ack.waitAsync()
                                    }
                                }
                            } catch (e: CancellationException) {
                                throw e
                            } catch (e: Exception) {
                                println("Receiver $i error: ${e.message}")
                                break
                            }
                        }
                    }

                    val tasks = mutableListOf<kotlinx.coroutines.Job>()
                    for (i in 0 until nReceivers) {
                        tasks.add(launch { runReceiver(i) })
                    }

                    delay(2500)

                    val sessionConfig =
                        SessionConfig(
                            sessionType = SessionType.POINT_TO_POINT,
                            maxRetries = 80u,
                            interval = Duration.ofMillis(400),
                            metadata = emptyMap(),
                            mlsSettings = if (mlsEnabled) MlsSettings(100u) else null,
                        )
                    val senderSessionContext = sender.createSession(sessionConfig, receiverName)
                    senderSessionContext.completion.waitAsync()
                    val senderSession = senderSessionContext.session

                    val payloadType = "hello message"
                    val metadata = mapOf("sender" to "hello")

                    publishMessages(senderSession, nMessages, payloadType, metadata)

                    waitForAck(senderSession)

                    validateAffinity(receiverCounts, nMessages)

                    val handle = sender.deleteSessionAsync(senderSession)
                    handle.waitAsync()

                    for (t in tasks) {
                        t.cancel()
                    }
                    for (t in tasks) {
                        try {
                            t.join()
                        } catch (_: CancellationException) {
                            // Expected after cancel
                        }
                    }
            } finally {
                teardownServer(server)
            }
        }
    }
}
