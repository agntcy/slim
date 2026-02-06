// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings

import kotlinx.coroutines.delay
import java.time.Duration

/**
 * Shared constants for tests.
 */
const val LONG_SECRET = "e4aaecb9ae0b23b82086bb8a8633e01fba16ae8d9c1379a613c00838"

/**
 * Server fixture for managing local and global service instances.
 * 
 * @param service The SLIM service instance
 * @param endpoint The endpoint string (null for global service)
 */
data class ServerFixture(
    val service: Service,
    val endpoint: String?
) {
    val localService: Boolean = endpoint != null
    
    /**
     * Get client configuration for connecting to this server.
     */
    fun getClientConfig(): ClientConfig? {
        return if (localService) {
            newInsecureClientConfig("http://$endpoint")
        } else {
            null
        }
    }
}

/**
 * Setup a server fixture with the given endpoint.
 * 
 * @param endpoint Endpoint string (e.g., "127.0.0.1:12345") or null for global service
 * @return ServerFixture instance
 */
suspend fun setupServer(endpoint: String?): ServerFixture {
    // Initialize global state
    val tracingConfig = newTracingConfig()
    val runtimeConfig = newRuntimeConfig()
    val serviceConfig = newServiceConfig()
    
    tracingConfig.logLevel = "info"
    initializeWithConfigs(
        tracingConfig = tracingConfig,
        runtimeConfig = runtimeConfig,
        serviceConfig = listOf(serviceConfig)
    )
    
    // Create service
    val service = if (endpoint != null) {
        val svc = Service("localserver")
        val serverConfig = newInsecureServerConfig(endpoint)
        svc.runServerAsync(serverConfig)
        svc
    } else {
        getGlobalService()
    }
    
    // Wait for server to start
    delay(1000)
    
    return ServerFixture(service, endpoint)
}

/**
 * Teardown the server fixture.
 */
suspend fun teardownServer(fixture: ServerFixture) {
    if (fixture.endpoint != null) {
        try {
            fixture.service.stopServer(fixture.endpoint)
        } catch (e: Exception) {
            println("Warning: error stopping server ${fixture.endpoint}: ${e.message}")
        }
    }
}

/**
 * Create a participant app with unique naming.
 * 
 * @param server Server fixture
 * @param testId Unique test identifier
 * @param index Participant index
 * @return Triple of (App, connection ID or null, participant name string)
 */
suspend fun createParticipant(
    server: ServerFixture,
    testId: String,
    index: Int
): Triple<App, ULong?, String> {
    val partName = "participant_$index"
    val name = Name("org", "test_$testId", partName)
    
    val connId: ULong? 
    val svc: Service
    
    if (server.localService) {
        svc = Service("svcparticipant$index")
        val clientConfig = server.getClientConfig()
        requireNotNull(clientConfig) { "Client config should not be null for local service" }
        connId = svc.connectAsync(clientConfig)
    } else {
        svc = server.service
        connId = null
    }
    
    val participant = svc.createAppWithSecret(name, LONG_SECRET)
    
    if (server.localService && connId != null) {
        val nameWithId = Name.newWithId("org", "test_$testId", partName, id = participant.id())
        participant.subscribeAsync(nameWithId, connId)
        delay(100)
    }
    
    return Triple(participant, connId, partName)
}
