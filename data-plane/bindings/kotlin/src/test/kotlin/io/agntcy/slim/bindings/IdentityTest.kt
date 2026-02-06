// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings

import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeout
import org.junit.jupiter.api.Test
import org.junit.jupiter.params.ParameterizedTest
import org.junit.jupiter.params.provider.ValueSource
import java.time.Duration
import kotlin.test.assertContentEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue
import kotlin.time.Duration.Companion.seconds

/**
 * JWT identity verification and shared secret validation tests.
 */
class IdentityTest {
    
    private val testAudience = listOf("test.audience")
    
    /**
     * Construct an App instance with a JWT identity provider/verifier.
     * 
     * @param name Name identifying this local app (used as JWT subject).
     * @param privateKey Path to PEM private key used for signing outbound tokens.
     * @param privateKeyAlgorithm Algorithm matching the private key type (e.g. ES256).
     * @param publicKey Path to PEM public key used to verify the peer's tokens.
     * @param publicKeyAlgorithm Algorithm matching the peer public key type.
     * @param wrongAudience Optional override audience list to force verification failure.
     *                      If null, uses the shared test_audience (success path).
     * @return App: A configured App instance.
     */
    private fun createSlim(
        name: Name,
        privateKey: String,
        privateKeyAlgorithm: JwtAlgorithm,
        publicKey: String,
        publicKeyAlgorithm: JwtAlgorithm,
        wrongAudience: List<String>? = null
    ): App {
        // Build signing key config (private key for encoding)
        val privateKeyConfig = JwtKeyConfig(
            algorithm = privateKeyAlgorithm,
            format = JwtKeyFormat.PEM,
            key = JwtKeyData.File(path = privateKey)
        )
        
        // Build verification key config (public key for decoding)
        val publicKeyConfig = JwtKeyConfig(
            algorithm = publicKeyAlgorithm,
            format = JwtKeyFormat.PEM,
            key = JwtKeyData.File(path = publicKey)
        )
        
        // Create provider config (for signing outbound tokens)
        val providerConfig = IdentityProviderConfig.Jwt(
            config = ClientJwtAuth(
                key = JwtKeyType.Encoding(key = privateKeyConfig),
                audience = testAudience,
                issuer = "test-issuer",
                subject = name.toString(),
                duration = Duration.ofSeconds(60)
            )
        )
        
        // Create verifier config (for verifying inbound tokens)
        val verifierConfig = IdentityVerifierConfig.Jwt(
            config = JwtAuth(
                key = JwtKeyType.Decoding(key = publicKeyConfig),
                audience = wrongAudience ?: testAudience,
                issuer = "test-issuer",
                subject = null,
                duration = Duration.ofSeconds(60)
            )
        )
        
        // Create and return the app
        return App(name, providerConfig, verifierConfig)
    }
    
    /**
     * End-to-end JWT identity verification test.
     * 
     * Parametrized:
     *     audience:
     *         - Matching audience list (expects successful request/reply)
     *         - Wrong audience list (expects receive timeout / verification failure)
     * 
     * Flow:
     *     1. Create sender & receiver Slim instances with distinct EC key pairs.
     *     2. Cross-wire each instance: each verifier trusts the other's public key.
     *     3. Establish route sender -> receiver.
     *     4. Sender creates PointToPoint session and publishes a request.
     *     5. Receiver listens, validates payload, replies.
     *     6. Validate response only when audience matches; otherwise expect timeout.
     * 
     * Assertions:
     *     - Payload integrity on both directions when audience matches.
     *     - Proper exception/timeout on audience mismatch.
     */
    @ParameterizedTest
    @ValueSource(strings = ["correct", "wrong"])
    fun testIdentityVerification(audienceType: String) = runTest(timeout = 30.seconds) {
        val server = setupServer(null) // Use global service
        
        try {
            // Initialize tracing and global state
            val tracingConfig = newTracingConfig()
            val runtimeConfig = newRuntimeConfig()
            val serviceConfig = newServiceConfig()
            
            tracingConfig.logLevel = "info"
            initializeWithConfigs(
                tracingConfig = tracingConfig,
                runtimeConfig = runtimeConfig,
                serviceConfig = listOf(serviceConfig)
            )
            
            val audience = if (audienceType == "correct") testAudience else listOf("wrong.audience")
            
            val senderName = Name("org", "default", "id_sender")
            val receiverName = Name("org", "default", "id_receiver")
            
            // Keys used for signing JWTs of sender
            val privateKeySender = "src/test/resources/testdata/ec256.pem"
            val publicKeySender = "src/test/resources/testdata/ec256-public.pem"
            val algorithmSender = JwtAlgorithm.ES256
            
            // Keys used for signing JWTs of receiver
            val privateKeyReceiver = "src/test/resources/testdata/ec384.pem"
            val publicKeyReceiver = "src/test/resources/testdata/ec384-public.pem"
            val algorithmReceiver = JwtAlgorithm.ES384
            
            // Create new app object. Note that the verifier will use the public key of the receiver
            // to verify the JWT of the reply message
            val appSender = createSlim(
                senderName,
                privateKeySender,
                algorithmSender,
                publicKeyReceiver,
                algorithmReceiver
            )
            
            // Create second local app. Note that the receiver will use the public key of the sender
            // to verify the JWT of the request message
            val appReceiver = createSlim(
                receiverName,
                privateKeyReceiver,
                algorithmReceiver,
                publicKeySender,
                algorithmSender,
                audience
            )
            
            // Create PointToPoint session
            val sessionConfig = SessionConfig(
                sessionType = SessionType.POINT_TO_POINT,
                enableMls = false,
                maxRetries = 3u,
                interval = Duration.ofMillis(333),
                metadata = emptyMap()
            )
            
            // Create session (returns immediately with session_context)
            val sessionContext = appSender.createSession(sessionConfig, receiverName)
            
            // Wait for session establishment based on audience
            if (audience == testAudience) {
                sessionContext.completion.waitAsync()
            } else {
                // Session establishment should timeout due to invalid audience
                assertFailsWith<Exception> {
                    withTimeout(3000) {
                        sessionContext.completion.waitAsync()
                    }
                }
            }
            
            // Messages
            val pubMsg = "thisistherequest".toByteArray()
            val resMsg = "thisistheresponse".toByteArray()
            
            // Test with reply - only when audience is correct
            if (audience == testAudience) {
                try {
                    /**
                     * Receiver side:
                     * - Wait for inbound session
                     * - Receive request
                     * - Reply with response payload
                     */
                    val backgroundTask = launch {
                        var recvSessionCtx: Session? = null
                        try {
                            recvSessionCtx = appReceiver.listenForSessionAsync(null)
                            val receivedMsg = recvSessionCtx.getMessageAsync(null)
                            val ctx = receivedMsg.context
                            val msgRcv = receivedMsg.payload
                            
                            // Make sure the message is correct
                            assertContentEquals(pubMsg, msgRcv)
                            
                            // Reply to the session
                            recvSessionCtx.publishToAsync(ctx, resMsg, null, null)
                        } catch (e: Exception) {
                            println("Error receiving message on app: ${e.message}")
                        }
                    }
                    
                    // As audience matches, we expect a successful request/reply
                    val handle = sessionContext.session.publishAsync(pubMsg, null, null)
                    handle.waitAsync()
                    val receivedMsg = sessionContext.session.getMessageAsync(null)
                    val message = receivedMsg.payload
                    
                    // Check if the message is correct
                    assertContentEquals(resMsg, message)
                    
                    // Wait for task to finish (and surface any exceptions)
                    backgroundTask.join()
                } finally {
                    // Delete sessions
                    val h = appSender.deleteSessionAsync(sessionContext.session)
                    h.waitAsync()
                }
            }
            
        } finally {
            teardownServer(server)
        }
    }
    
    /**
     * Test that creating an app with too short shared secret raises an exception.
     */
    @Test
    fun testInvalidSharedSecretTooShort() {
        val name = Name("org", "default", "test_app")
        
        // Create app with a secret that's too short (minimum is 32 characters)
        val shortSecret = "tooshort"
        
        // Get the global service
        val service = getGlobalService()
        
        // Should raise an exception when creating the app
        val exception = assertFailsWith<Exception> {
            service.createAppWithSecret(name, shortSecret)
        }
        
        // Verify the error message mentions the secret being too short
        assertTrue(
            exception.message?.contains("short", ignoreCase = true) == true ||
            exception.message?.contains("invalid", ignoreCase = true) == true
        )
    }
    
    /**
     * Test that creating an app with empty shared secret raises an exception.
     */
    @Test
    fun testInvalidSharedSecretEmpty() {
        val name = Name("org", "default", "test_app")
        
        // Empty secret
        val emptySecret = ""
        
        // Get the global service
        val service = getGlobalService()
        
        // Should raise an exception when creating the app
        val exception = assertFailsWith<Exception> {
            service.createAppWithSecret(name, emptySecret)
        }
        
        // Verify the error message is appropriate
        assertTrue(
            exception.message?.contains("short", ignoreCase = true) == true ||
            exception.message?.contains("invalid", ignoreCase = true) == true
        )
    }
}
