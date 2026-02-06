// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings

import kotlinx.coroutines.delay
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeout
import org.junit.jupiter.api.Test
import org.junit.jupiter.params.ParameterizedTest
import org.junit.jupiter.params.provider.ValueSource
import java.time.Duration
import kotlin.test.*
import kotlin.time.Duration.Companion.seconds

/**
 * Integration tests for the slim_bindings Kotlin layer.
 * 
 * These tests exercise:
 * - End-to-end PointToPoint session creation, message publish/reply, and cleanup
 * - Session configuration retrieval and default session configuration propagation
 * - Automatic client reconnection after a server restart
 * - Error handling when targeting a non-existent subscription
 */
class BindingsTest {
    
    /**
     * Full round-trip:
     * - Two services connect (Alice, Bob)
     * - Subscribe & route setup
     * - PointToPoint session creation (Alice -> Bob)
     * - Publish + receive + reply
     * - Validate session IDs, payload integrity
     * - Test error behavior after deleting session
     * - Disconnect cleanup
     */
    @Test
    fun testEndToEnd() = runTest(timeout = 60.seconds) {
        val server = setupServer("127.0.0.1:12344")
        
        try {
            val aliceName = Name("org", "default", "alice_e2e")
            val bobName = Name("org", "default", "bob_e2e")
            
            // Connect to the service
            var connIdAlice: ULong? = null
            var connIdBob: ULong? = null
            val svcAlice: Service
            val svcBob: Service
            
            if (server.localService) {
                svcAlice = Service("svcalice")
                svcBob = Service("svcbob")
                
                connIdAlice = svcAlice.connectAsync(server.getClientConfig()!!)
                connIdBob = svcBob.connectAsync(server.getClientConfig()!!)
            } else {
                svcAlice = server.service
                svcBob = server.service
            }
            
            // Create 2 apps, Alice and Bob
            val appAlice = svcAlice.createAppWithSecret(aliceName, LONG_SECRET)
            val appBob = svcBob.createAppWithSecret(bobName, LONG_SECRET)
            
            var aliceNameFinal = aliceName
            var bobNameFinal = bobName
            
            if (server.localService) {
                // Subscribe alice and bob
                aliceNameFinal = Name.newWithId("org", "default", "alice_e2e", id = appAlice.id())
                bobNameFinal = Name.newWithId("org", "default", "bob_e2e", id = appBob.id())
                
                appAlice.subscribeAsync(aliceNameFinal, connIdAlice!!)
                appBob.subscribeAsync(bobNameFinal, connIdBob!!)
                
                delay(1000)
                
                // Set routes
                appAlice.setRouteAsync(bobNameFinal, connIdAlice)
            }
            
            delay(1000)
            println(aliceNameFinal)
            println(bobNameFinal)
            
            // Create point to point session
            val sessionContextAlice = appAlice.createSession(
                SessionConfig(
                    sessionType = SessionType.POINT_TO_POINT,
                    enableMls = false,
                    maxRetries = 5u,
                    interval = Duration.ofSeconds(1),
                    metadata = emptyMap()
                ),
                bobNameFinal
            )
            
            // Wait for the session handshake to fully complete
            sessionContextAlice.completion.waitAsync()
            
            // Send msg from Alice to Bob
            val msg = byteArrayOf(1, 2, 3, 4, 5, 6, 7, 8, 9)
            val handle = sessionContextAlice.session.publishAsync(msg, null, null)
            
            // Wait for ack to be received
            handle.waitAsync()
            
            // Receive session from Alice
            val sessionContextBob = appBob.listenForSessionAsync(timeout = Duration.ofSeconds(5))
            
            // Receive message from Alice
            val receivedMsg = sessionContextBob.getMessageAsync(timeout = Duration.ofSeconds(5))
            val messageCtx = receivedMsg.context
            val msgRcv = receivedMsg.payload
            
            // Make sure the session id corresponds
            assertEquals(sessionContextBob.sessionId(), sessionContextAlice.session.sessionId())
            
            // Check if the message is correct
            assertContentEquals(msg, msgRcv)
            
            // Reply to Alice
            val handleReply = sessionContextBob.publishToAsync(messageCtx, msgRcv, null, null)
            
            // Wait for ack
            handleReply.waitAsync()
            
            // Wait for message
            val receivedMsgAlice = sessionContextAlice.session.getMessageAsync(Duration.ofSeconds(5))
            val msgRcvAlice = receivedMsgAlice.payload
            
            // Check if the message is correct
            assertContentEquals(msg, msgRcvAlice)
            
            // Delete both sessions by deleting alice's session
            appAlice.deleteSessionAsync(sessionContextAlice.session)
            
            // Try to send a message after deleting the session - this should raise an exception
            val exception = assertFailsWith<Exception> {
                sessionContextAlice.session.publishAsync(msg, null, null)
            }
            assertTrue(exception.message?.contains("closed", ignoreCase = true) == true,
                "Unexpected error message: ${exception.message}")
            
            if (connIdAlice != null) {
                // Disconnect alice
                svcAlice.disconnect(connIdAlice)
            }
            
            val exception2 = assertFailsWith<Exception> {
                appAlice.deleteSessionAsync(sessionContextAlice.session)
            }
            assertTrue(exception2.message?.contains("closed", ignoreCase = true) == true)
            
        } finally {
            teardownServer(server)
        }
    }
    
    /**
     * Test resilience / auto-reconnect:
     * - Establish connection and session
     * - Exchange a baseline message
     * - Stop and restart server
     * - Wait for automatic reconnection
     * - Publish again and confirm continuity using original session context
     */
    @Test
    fun testAutoReconnectAfterServerRestart() = runTest(timeout = 90.seconds) {
        val endpoint = "127.0.0.1:12346"
        
        val svcServer = Service("svcserver")
        val svcAlice = Service("svcalice")
        val svcBob = Service("svcbob")
        
        val serverConf = newInsecureServerConfig(endpoint)
        svcServer.runServerAsync(serverConf)
        
        val clientConf = newInsecureClientConfig("http://$endpoint")
        val connIdAlice = svcAlice.connectAsync(clientConf)
        val connIdBob = svcBob.connectAsync(clientConf)
        
        val aliceName = Name("org", "default", "alice_res")
        val bobName = Name("org", "default", "bob_res")
        
        // Create 2 apps, Alice and Bob
        val appAlice = svcAlice.createAppWithSecret(aliceName, LONG_SECRET)
        val appBob = svcBob.createAppWithSecret(bobName, LONG_SECRET)
        
        // Subscribe alice and bob
        val aliceNameWithId = Name.newWithId("org", "default", "alice_res", id = appAlice.id())
        val bobNameWithId = Name.newWithId("org", "default", "bob_res", id = appBob.id())
        
        appAlice.subscribeAsync(aliceNameWithId, connIdAlice)
        appBob.subscribeAsync(bobNameWithId, connIdBob)
        
        delay(1000)
        
        // Set routes
        appAlice.setRouteAsync(bobNameWithId, connIdAlice)
        
        delay(1000)
        
        // Create point to point session
        val sessionContextAlice = appAlice.createSession(
            SessionConfig(
                sessionType = SessionType.POINT_TO_POINT,
                enableMls = false,
                maxRetries = 5u,
                interval = Duration.ofSeconds(1),
                metadata = emptyMap()
            ),
            bobNameWithId
        )
        
        // Wait for the session handshake to fully complete
        sessionContextAlice.completion.waitAsync()
        
        // Send baseline message Alice -> Bob; Bob should first receive a new session then the message
        val baselineMsg = byteArrayOf(1, 2, 3)
        val handle = sessionContextAlice.session.publishAsync(baselineMsg, null, null)
        handle.waitAsync()
        
        // Bob waits for new session
        val bobSessionCtx = appBob.listenForSessionAsync(timeout = Duration.ofSeconds(5))
        val receivedMsg = bobSessionCtx.getMessageAsync(timeout = Duration.ofSeconds(5))
        val received = receivedMsg.payload
        assertContentEquals(baselineMsg, received)
        
        // Session ids should match
        assertEquals(bobSessionCtx.sessionId(), sessionContextAlice.session.sessionId())
        
        // Stop the server
        svcServer.shutdownAsync()
        
        delay(3000) // Allow time for the server to fully shut down
        
        // Start the server again - reuse existing service instead of creating new one
        svcServer.runServerAsync(newInsecureServerConfig(endpoint))
        delay(5000) // Allow more time for server to restart and clients to reconnect
        
        // Test that the message exchange resumes normally after the simulated restart
        val testMsg = byteArrayOf(4, 5, 6)
        val handle2 = sessionContextAlice.session.publishAsync(testMsg, null, null)
        handle2.waitAsync()
        
        // Bob should still use the existing session context; just receive next message
        val receivedMsg2 = bobSessionCtx.getMessageAsync(timeout = Duration.ofSeconds(5))
        val received2 = receivedMsg2.payload
        assertContentEquals(testMsg, received2)
        
        // Delete sessions by deleting alice session
        appAlice.deleteSessionAsync(sessionContextAlice.session)
        
        // Clean up
        svcAlice.disconnect(connIdAlice)
        svcBob.disconnect(connIdBob)
        svcServer.shutdownAsync()
    }
    
    /**
     * Validate error path when publishing to an unsubscribed / nonexistent destination:
     * - Create only Alice, subscribe her
     * - Publish message addressed to Bob (not connected)
     * - Expect an error surfaced (no matching subscription)
     */
    @Test
    fun testErrorOnNonexistentSubscription() = runTest(timeout = 30.seconds) {
        val server = setupServer("127.0.0.1:12347")
        
        try {
            val aliceName = Name("org", "default", "alice_nonsub")
            
            var connIdAlice: ULong? = null
            val svcAlice: Service
            val appAlice: App
            
            if (server.localService) {
                svcAlice = Service("svcalice")
                
                // Connect client and subscribe for messages
                connIdAlice = svcAlice.connectAsync(server.getClientConfig()!!)
                
                appAlice = svcAlice.createAppWithSecret(aliceName, LONG_SECRET)
                
                val aliceNameWithId = Name.newWithId("org", "default", "alice_nonsub", id = appAlice.id())
                appAlice.subscribeAsync(aliceNameWithId, connIdAlice)
            } else {
                svcAlice = server.service
                appAlice = svcAlice.createAppWithSecret(aliceName, LONG_SECRET)
            }
            
            // Create Bob's name, but do not instantiate or subscribe Bob
            val bobName = Name("org", "default", "bob_nonsub")
            
            // Create point to point session (Alice only) - should timeout or error since Bob is not there
            assertFailsWith<Exception> {
                val session = appAlice.createSession(
                    SessionConfig(
                        sessionType = SessionType.POINT_TO_POINT,
                        enableMls = false,
                        maxRetries = null,
                        interval = null,
                        metadata = emptyMap()
                    ),
                    bobName
                )
                
                withTimeout(10000) {
                    session.completion.waitAsync()
                }
            }
            
            // Clean up
            if (connIdAlice != null) {
                svcAlice.disconnect(connIdAlice)
            }
            
        } finally {
            teardownServer(server)
        }
    }
    
    /**
     * Test that listen_for_session times out appropriately when no session is available.
     */
    @ParameterizedTest
    @ValueSource(strings = ["127.0.0.1:12348", ""])
    fun testListenForSessionTimeout(endpoint: String) = runTest(timeout = 30.seconds) {
        val server = setupServer(if (endpoint.isEmpty()) null else endpoint)
        
        try {
            val aliceName = Name("org", "default", "alice_timeout")
            
            var connIdAlice: ULong? = null
            val svcAlice: Service
            val appAlice: App
            
            if (server.localService) {
                svcAlice = Service("svcalice")
                
                // Connect to the service to get connection ID
                connIdAlice = svcAlice.connectAsync(server.getClientConfig()!!)
                
                appAlice = svcAlice.createAppWithSecret(aliceName, LONG_SECRET)
                
                val aliceNameWithId = Name.newWithId("org", "default", "alice_timeout", id = appAlice.id())
                appAlice.subscribeAsync(aliceNameWithId, connIdAlice)
            } else {
                svcAlice = server.service
                appAlice = svcAlice.createAppWithSecret(aliceName, LONG_SECRET)
            }
            
            // Test with a short timeout - should raise an exception
            val startTime = System.currentTimeMillis()
            val timeout = Duration.ofMillis(100)
            
            val exception = assertFailsWith<Exception> {
                appAlice.listenForSessionAsync(timeout = timeout)
            }
            
            val elapsedTime = (System.currentTimeMillis() - startTime) / 1000.0
            
            // Verify the timeout was respected (allow some tolerance)
            assertTrue(elapsedTime >= 0.08 && elapsedTime <= 0.2,
                "Timeout took ${elapsedTime}s, expected ~0.1s")
            assertTrue(
                exception.message?.contains("timed out", ignoreCase = true) == true ||
                exception.message?.contains("timeout", ignoreCase = true) == true
            )
            
            // Test with None timeout - should wait indefinitely (we'll interrupt it)
            assertFailsWith<Exception> {
                withTimeout(100) {
                    appAlice.listenForSessionAsync(timeout = null)
                }
            }
            
            // Clean up
            if (connIdAlice != null) {
                svcAlice.disconnect(connIdAlice)
            }
            
        } finally {
            teardownServer(server)
        }
    }
    
    /**
     * Test that get_message times out appropriately when no message is available.
     */
    @ParameterizedTest
    @ValueSource(strings = ["127.0.0.1:12349", ""])
    fun testGetMessageTimeout(endpoint: String) = runTest(timeout = 60.seconds) {
        val server = setupServer(if (endpoint.isEmpty()) null else endpoint)
        
        try {
            val aliceName = Name("org", "default", "alice_msg_timeout")
            val bobName = Name("org", "default", "bob_msg_timeout")
            
            var connIdAlice: ULong? = null
            var connIdBob: ULong? = null
            val svcAlice: Service
            val svcBob: Service
            
            if (server.localService) {
                svcAlice = Service("svcalice")
                svcBob = Service("svcbob")
                
                connIdAlice = svcAlice.connectAsync(server.getClientConfig()!!)
                connIdBob = svcBob.connectAsync(server.getClientConfig()!!)
            } else {
                svcAlice = server.service
                svcBob = server.service
            }
            
            // Create 2 apps, Alice and Bob
            val appAlice = svcAlice.createAppWithSecret(aliceName, LONG_SECRET)
            val appBob = svcBob.createAppWithSecret(bobName, LONG_SECRET)
            
            var aliceNameFinal = aliceName
            var bobNameFinal = bobName
            
            if (server.localService) {
                // Subscribe alice and bob
                aliceNameFinal = Name.newWithId("org", "default", "alice_msg_timeout", id = appAlice.id())
                bobNameFinal = Name.newWithId("org", "default", "bob_msg_timeout", id = appBob.id())
                
                appAlice.subscribeAsync(aliceNameFinal, connIdAlice!!)
                appBob.subscribeAsync(bobNameFinal, connIdBob!!)
                
                delay(1000)
                
                // Set routes
                appAlice.setRouteAsync(bobNameFinal, connIdAlice)
            }
            
            delay(1000)
            
            // Create point to point session
            val sessionContextAlice = appAlice.createSession(
                SessionConfig(
                    sessionType = SessionType.POINT_TO_POINT,
                    enableMls = false,
                    maxRetries = 5u,
                    interval = Duration.ofSeconds(1),
                    metadata = emptyMap()
                ),
                bobNameFinal
            )
            
            // Wait for the session handshake to fully complete
            sessionContextAlice.completion.waitAsync()
            
            // Bob waits for new session
            val bobSessionCtx = appBob.listenForSessionAsync(timeout = Duration.ofSeconds(5))
            
            // Test with a short timeout - should raise an exception when no message is available
            val startTime = System.currentTimeMillis()
            val timeout = Duration.ofMillis(100)
            
            val exception = assertFailsWith<Exception> {
                bobSessionCtx.getMessageAsync(timeout = timeout)
            }
            
            val elapsedTime = (System.currentTimeMillis() - startTime) / 1000.0
            
            // Verify the timeout was respected (allow some tolerance)
            assertTrue(elapsedTime >= 0.08 && elapsedTime <= 0.2,
                "Timeout took ${elapsedTime}s, expected ~0.1s")
            assertTrue(
                exception.message?.contains("timed out", ignoreCase = true) == true ||
                exception.message?.contains("timeout", ignoreCase = true) == true
            )
            
            // Test with None timeout - should wait indefinitely (we'll interrupt it)
            assertFailsWith<Exception> {
                withTimeout(100) {
                    bobSessionCtx.getMessageAsync(timeout = null)
                }
            }
            
            // Delete sessions
            val handleAlice = appAlice.deleteSessionAsync(sessionContextAlice.session)
            val handleBob = appBob.deleteSessionAsync(bobSessionCtx)
            
            // Wait for deletion
            handleAlice.waitAsync()
            handleBob.waitAsync()
            
            // Clean up
            if (connIdAlice != null) {
                svcAlice.disconnect(connIdAlice)
            }
            if (connIdBob != null) {
                svcBob.disconnect(connIdBob)
            }
            
        } finally {
            teardownServer(server)
        }
    }
}
