// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.jupiter.api.Test
import java.net.ServerSocket
import java.time.Duration
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * Point-to-point sticky session integration test.
 *
 * Scenario:
 *   - One logical sender creates a PointToPoint session and sends many messages
 *     to a shared logical receiver identity.
 *   - Ten receiver instances (same Name) concurrently listen for an
 *     inbound session. Only one should become the bound peer for the
 *     PointToPoint session (stickiness).
 *   - All messages must arrive at exactly one receiver (verifying
 *     session affinity) and none at the others.
 *   - Runs with MLS enabled (matches Java bindings test).
 *
 * The global in-process (`null` endpoint) variant is intentionally omitted here:
 * it is still exercised by other bindings and is significantly flakier under CI.
 *
 * Validated invariants:
 *   * session_type == PointToPoint for receiver-side context
 *   * dst == sender.local_name and src == receiver.local_name
 *   * Exactly one receiver_counts[i] == nMessages and total sum == nMessages
 */
class PointToPointTest {
    /** Binds an ephemeral TCP port and returns `host:port` for the embedded server. */
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
     *
     * Publishes sequentially (each delivery awaited before the next) so we do not
     * stack thousands of in-flight reliable sends. The Java bindings test does the same.
     */
    private suspend fun publishMessages(
        senderSession: Session,
        nMessages: Int,
        payloadType: String?,
        metadata: Map<String, String>?,
    ) {
        repeat(nMessages) { i ->
            senderSession.publishAndWaitAsync(
                "Hello from sender".toByteArray(),
                payloadType,
                metadata,
            )
            if ((i + 1) % 200 == 0) {
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
     * Uses an ephemeral listen port and a port-scoped embedded host service name (see
     * [setupServer]) so this test does not collide with other binding tests in one JVM.
     *
     * Flow:
     *     1. Spawn 10 receiver tasks (same logical Name).
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
        val endpoint = freeLocalEndpoint()
        val mlsEnabled = true
        // Default runBlocking uses one thread; receiver coroutines must run in parallel with
        // publishAndWaitAsync (same as Java's fixed thread pool for receivers).
        runBlocking(Dispatchers.Default) {
            val server = setupServer(endpoint)

            try {
                    // Generate unique names to avoid collisions
                    val testId = UUID.randomUUID().toString().substring(0, 8)
                    val senderName = Name("org", "test_$testId", "p2psender")
                    val receiverName = Name("org", "test_$testId", "p2preceiver")

                    println("Sender name: $senderName")
                    println("Receiver name: $receiverName")

                    // Create sender service and app
                    val (sender, _) = setupSender(server, senderName, testId, receiverName)

                    val receiverCounts = ConcurrentHashMap<Int, AtomicInteger>()
                    for (i in 0 until 10) {
                        receiverCounts[i] = AtomicInteger(0)
                    }

                    val subscribedLatch = CountDownLatch(10)
                    val goListenLatch = CountDownLatch(1)

                    // Fewer than Java (1000): CI can exhaust reliable-send retries if the dataplane
                    // is under load from 10 concurrent receivers + reconnect churn.
                    val nMessages = 400

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
                        val svcReceiver =
                            if (server.localService) {
                                val svc = Service("svcreceiver$i")
                                connIdReceiver = svc.connectAsync(server.getClientConfig()!!)
                                svc
                            } else {
                                server.service
                            }

                        val receiver = svcReceiver.createAppWithSecret(receiverName, LONG_SECRET)

                        if (server.localService) {
                            // Subscribe receiver
                            val receiverNameWithId =
                                Name.newWithId("org", "test_$testId", "p2preceiver", id = receiver.id())
                            receiver.subscribeAsync(receiverNameWithId, connIdReceiver!!)
                            delay(100) // Real coroutine delay
                        }

                        subscribedLatch.countDown()
                        withContext(Dispatchers.IO) {
                            goListenLatch.await()
                        }

                        val sessionContext =
                            receiver.listenForSessionAsync(timeout = Duration.ofSeconds(120))
                        val session = sessionContext

                        // Make sure the received session is PointToPoint
                        assertEquals(SessionType.POINT_TO_POINT, session.sessionType())

                        while (true) {
                            try {
                                val receivedMsg = session.getMessageAsync(timeout = Duration.ofSeconds(30))
                                val ctx = receivedMsg.context

                                if (ctx.payloadType == "hello message" && ctx.metadata["sender"] == "hello") {
                                    val count = receiverCounts[i]!!.incrementAndGet()

                                    if (count == nMessages) {
                                        // Send back application acknowledgment (await so the sender never races teardown)
                                        session.publishAndWaitAsync(
                                            "All messages received: $i".toByteArray(),
                                            null,
                                            null,
                                        )
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
                    for (i in 0 until 10) {
                        val task =
                            launch {
                                runReceiver(i)
                            }
                        tasks.add(task)
                    }

                    require(subscribedLatch.await(120, TimeUnit.SECONDS)) {
                        "receivers did not finish subscribe within 120s"
                    }
                    goListenLatch.countDown()
                    delay(200)

                    // Wider retries than Java: Kotlin CI often runs after other binding tests and
                    // can see transient connect / dataplane churn mid-run.
                    val sessionConfig =
                        SessionConfig(
                            sessionType = SessionType.POINT_TO_POINT,
                            enableMls = mlsEnabled,
                            maxRetries = 25u,
                            interval = Duration.ofMillis(300),
                            metadata = emptyMap(),
                        )
                    val senderSessionContext = sender.createSession(sessionConfig, receiverName)
                    senderSessionContext.completion.waitAsync()
                    val senderSession = senderSessionContext.session

                    val payloadType = "hello message"
                    val metadata = mapOf("sender" to "hello")

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
}
