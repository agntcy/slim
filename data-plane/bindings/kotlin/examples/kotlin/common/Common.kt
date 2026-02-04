// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples.common

import io.agntcy.slim.bindings.*
import java.io.File
import java.time.Duration
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

/**
 * Shared helper utilities for the slim_bindings Kotlin examples.
 * 
 * This module centralizes:
 *  - Pretty-print / color formatting helpers
 *  - Identity (auth) helper constructors (shared secret / JWT / SPIRE)
 *  - Convenience function for constructing a local Slim app using global service
 *  - Configuration parsing utilities
 */

/**
 * Split an ID of form organization/namespace/application (or channel).
 * 
 * @param id String in the canonical 'org/namespace/app-or-stream' format
 * @return Name: Constructed identity object
 * @throws IllegalArgumentException if the id cannot be split into exactly three segments
 */
fun splitId(id: String): Name {
    val parts = id.split("/")
    require(parts.size == 3) { 
        "Error: IDs must be in the format organization/namespace/app-or-stream. Got: $id" 
    }
    return Name(parts[0], parts[1], parts[2])
}

/**
 * Construct a JWT provider and verifier from file inputs.
 * 
 * Process:
 *   1. Read a JSON structure containing (base64-encoded) JWKS data (a SPIRE
 *      bundle with a JWKS for each trust domain).
 *   2. Decode & merge all JWKS entries.
 *   3. Create a JWT identity provider with static file JWT.
 *   4. Wrap merged JWKS JSON as JwtKeyConfig with RS256 & JWKS format.
 *   5. Build a JWT verifier using the JWKS-derived public key.
 */
fun jwtIdentity(
    jwtPath: String,
    spireBundlePath: String,
    localName: String,
    iss: String? = null,
    sub: String? = null,
    aud: List<String>? = null
): Pair<IdentityProviderConfig, IdentityVerifierConfig> {
    println("Using SPIRE bundle file: $spireBundlePath")
    
    val spireBundleString = File(spireBundlePath).readText()
    val spireBundle = Json.parseToJsonElement(spireBundleString) as JsonObject
    
    val allKeys = mutableListOf<JsonElement>()
    spireBundle.forEach { (trustDomain, v) ->
        println("Processing trust domain: $trustDomain")
        try {
            val decodedJwks = java.util.Base64.getDecoder().decode(v.toString().trim('"'))
            val jwksJson = Json.parseToJsonElement(String(decodedJwks)) as JsonObject
            
            jwksJson["keys"]?.let { keys ->
                val keyList = keys as JsonArray
                allKeys.addAll(keyList)
                println("  Added ${keyList.size} keys from $trustDomain")
            } ?: println("  Warning: No 'keys' found in JWKS for $trustDomain")
        } catch (e: Exception) {
            throw RuntimeException("Failed to process trust domain $trustDomain: ${e.message}", e)
        }
    }
    
    val spireJwks = """{"keys":${Json.encodeToString(JsonArray.serializer(), JsonArray(allKeys))}}"""
    
    println("Combined JWKS contains ${allKeys.size} total keys from ${spireBundle.size} trust domains")
    
    // Read the static JWT file for signing
    val jwtContent = File(jwtPath).readText()
    
    // Create encoding key config for JWT signing
    val encodingKeyConfig = JwtKeyConfig(
        algorithm = JwtAlgorithm.RS256,
        format = JwtKeyFormat.PEM,
        key = JwtKeyData.Data(value = jwtContent)
    )
    
    // Create provider config for JWT authentication
    val providerConfig = IdentityProviderConfig.Jwt(
        config = ClientJwtAuth(
            key = JwtKeyType.Encoding(key = encodingKeyConfig),
            audience = aud ?: listOf("default-audience"),
            issuer = iss ?: "default-issuer",
            subject = sub ?: localName,
            duration = Duration.ofSeconds(3600)
        )
    )
    
    // Create decoding key config for JWKS verification
    val decodingKeyConfig = JwtKeyConfig(
        algorithm = JwtAlgorithm.RS256,
        format = JwtKeyFormat.JWKS,
        key = JwtKeyData.Data(value = spireJwks)
    )
    
    // Create verifier config
    val verifierConfig = IdentityVerifierConfig.Jwt(
        config = JwtAuth(
            key = JwtKeyType.Decoding(key = decodingKeyConfig),
            audience = aud ?: listOf("default-audience"),
            issuer = iss ?: "default-issuer",
            subject = sub,
            duration = Duration.ofSeconds(3600)
        )
    )
    
    return Pair(providerConfig, verifierConfig)
}

/**
 * Construct a SPIRE-based dynamic identity provider and verifier.
 * 
 * @param socketPath SPIRE Workload API socket path (optional)
 * @param targetSpiffeId Specific SPIFFE ID to request (optional)
 * @param jwtAudiences Audience list for JWT SVID requests (optional)
 */
fun spireIdentity(
    socketPath: String?,
    targetSpiffeId: String?,
    jwtAudiences: List<String>?
): Pair<IdentityProviderConfig, IdentityVerifierConfig> {
    val spireConfig = SpireConfig(
        trustDomains = emptyList(),
        socketPath = socketPath,
        targetSpiffeId = targetSpiffeId,
        jwtAudiences = jwtAudiences ?: emptyList()
    )
    
    val providerConfig = IdentityProviderConfig.Spire(config = spireConfig)
    val verifierConfig = IdentityVerifierConfig.Spire(config = spireConfig)
    
    return Pair(providerConfig, verifierConfig)
}

/**
 * Initialize tracing and global service.
 * 
 * @param enableOpentelemetry Enable OpenTelemetry tracing
 * @return Service instance
 */
fun setupService(enableOpentelemetry: Boolean = false): Service {
    // Initialize tracing and global state
    val tracingConfig = newTracingConfig()
    val runtimeConfig = newRuntimeConfig()
    val serviceConfig = newServiceConfig()
    
    tracingConfig.logLevel = "info"
    
    if (enableOpentelemetry) {
        // Note: OpenTelemetry configuration through config objects is complex
        // For now, we'll just initialize with default tracing
        // Users can set OTEL environment variables for full OTEL support
    }
    
    initializeWithConfigs(
        tracingConfig = tracingConfig,
        runtimeConfig = runtimeConfig,
        serviceConfig = listOf(serviceConfig)
    )
    
    // Get the global service instance
    return getGlobalService()
}

/**
 * Build a Slim application instance using the global service.
 * 
 * Resolution precedence for auth:
 *   1. If SPIRE options provided -> SPIRE dynamic identity flow.
 *   2. Else if jwt + bundle + audience provided -> JWT/JWKS flow.
 *   3. Else -> shared secret (must be provided).
 * 
 * @param config BaseConfig instance containing all configuration
 * @return Pair of App instance and connection ID
 */
suspend fun createLocalApp(config: BaseConfig): Pair<App, ULong> = coroutineScope {
    // Initialize tracing and global state
    val service = setupService(config.enableOpentelemetry)
    
    // Convert local identifier to a strongly typed Name
    val localName = splitId(config.local)
    
    val clientConfig = newInsecureClientConfig(config.slim)
    val connId = service.connectAsync(clientConfig)
    
    // Determine authentication mode
    val authMode = config.getAuthMode()
    
    val localApp = when (authMode) {
        AuthMode.SPIRE -> {
            println("Using SPIRE dynamic identity authentication.")
            val (providerConfig, verifierConfig) = spireIdentity(
                socketPath = config.spireSocketPath,
                targetSpiffeId = config.spireTargetSpiffeId,
                jwtAudiences = config.spireJwtAudience
            )
            service.createApp(localName, providerConfig, verifierConfig)
        }
        AuthMode.JWT -> {
            println("Using JWT + JWKS authentication.")
            require(config.jwt != null && config.spireTrustBundle != null) {
                "JWT and SPIRE trust bundle are required for JWT auth mode"
            }
            val (providerConfig, verifierConfig) = jwtIdentity(
                config.jwt!!,
                config.spireTrustBundle!!,
                localName.toString(),
                aud = config.audience
            )
            service.createApp(localName, providerConfig, verifierConfig)
        }
        AuthMode.SHARED_SECRET -> {
            println("Using shared-secret authentication.")
            service.createAppWithSecret(localName, config.sharedSecret)
        }
    }
    
    // Provide feedback to user (instance numeric id)
    Colors.printFormatted("${localApp.id()}", "Created app")
    
    // Subscribe to the local name
    localApp.subscribeAsync(localName, connId)
    
    Pair(localApp, connId)
}

/**
 * Extension function to convert Name to string representation.
 */
fun Name.toIdString(): String {
    val parts = this.components()
    return "${parts[0]}/${parts[1]}/${parts[2]}"
}
