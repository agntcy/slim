// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples.common

import java.io.File
import java.nio.file.Path
import kotlin.io.path.exists

/**
 * Authentication mode for SLIM applications.
 */
enum class AuthMode {
    SHARED_SECRET,
    JWT,
    SPIRE
}

/**
 * Base configuration shared across all SLIM examples.
 * 
 * Environment variables are prefixed with SLIM_ by default.
 * Example: SLIM_LOCAL=org/ns/app
 */
open class BaseConfig(
    // Core identity settings
    val local: String,
    val remote: String? = null,
    
    // Service connection
    val slim: String = System.getenv("SLIM_ENDPOINT") ?: "http://127.0.0.1:46357",
    
    // Feature flags
    val enableOpentelemetry: Boolean = System.getenv("SLIM_ENABLE_OPENTELEMETRY")?.toBoolean() ?: false,
    val enableMls: Boolean = false,
    
    // Shared secret authentication (default, not for production)
    val sharedSecret: String = System.getenv("SLIM_SHARED_SECRET") ?: "abcde-12345-fedcb-67890-deadc",
    
    // JWT authentication
    val jwt: String? = System.getenv("SLIM_JWT"),
    val spireTrustBundle: String? = System.getenv("SLIM_SPIRE_TRUST_BUNDLE"),
    val audience: List<String>? = System.getenv("SLIM_AUDIENCE")?.split(",")?.map { it.trim() },
    
    // SPIRE dynamic identity
    val spireSocketPath: String? = System.getenv("SLIM_SPIRE_SOCKET_PATH"),
    val spireTargetSpiffeId: String? = System.getenv("SLIM_SPIRE_TARGET_SPIFFE_ID"),
    val spireJwtAudience: List<String>? = System.getenv("SLIM_SPIRE_JWT_AUDIENCE")?.split(",")?.map { it.trim() }
) {
    init {
        require(local.isNotEmpty()) { "Local ID is required" }
        
        // Validate file paths if provided
        jwt?.let { path ->
            require(File(path).exists()) { "JWT file not found: $path" }
        }
        spireTrustBundle?.let { path ->
            require(File(path).exists()) { "SPIRE trust bundle file not found: $path" }
        }
    }
    
    /**
     * Determine which authentication mode to use based on provided config.
     */
    fun getAuthMode(): AuthMode {
        return when {
            spireSocketPath != null || spireTargetSpiffeId != null || spireJwtAudience != null -> AuthMode.SPIRE
            jwt != null && spireTrustBundle != null && audience != null -> AuthMode.JWT
            else -> AuthMode.SHARED_SECRET
        }
    }
}

/**
 * Configuration specific to point-to-point messaging examples.
 */
class PointToPointConfig(
    local: String,
    remote: String? = null,
    slim: String = "http://127.0.0.1:46357",
    enableOpentelemetry: Boolean = false,
    enableMls: Boolean = false,
    sharedSecret: String = "abcde-12345-fedcb-67890-deadc",
    jwt: String? = null,
    spireTrustBundle: String? = null,
    audience: List<String>? = null,
    spireSocketPath: String? = null,
    spireTargetSpiffeId: String? = null,
    spireJwtAudience: List<String>? = null,
    
    // P2P specific
    val message: String? = null,
    val iterations: Int = 10
) : BaseConfig(
    local = local,
    remote = remote,
    slim = slim,
    enableOpentelemetry = enableOpentelemetry,
    enableMls = enableMls,
    sharedSecret = sharedSecret,
    jwt = jwt,
    spireTrustBundle = spireTrustBundle,
    audience = audience,
    spireSocketPath = spireSocketPath,
    spireTargetSpiffeId = spireTargetSpiffeId,
    spireJwtAudience = spireJwtAudience
) {
    init {
        require(iterations > 0) { "Iterations must be greater than 0" }
    }
}

/**
 * Configuration specific to group messaging examples.
 */
class GroupConfig(
    local: String,
    remote: String? = null,
    slim: String = "http://127.0.0.1:46357",
    enableOpentelemetry: Boolean = false,
    enableMls: Boolean = false,
    sharedSecret: String = "abcde-12345-fedcb-67890-deadc",
    jwt: String? = null,
    spireTrustBundle: String? = null,
    audience: List<String>? = null,
    spireSocketPath: String? = null,
    spireTargetSpiffeId: String? = null,
    spireJwtAudience: List<String>? = null,
    
    // Group specific
    val invites: List<String>? = null
) : BaseConfig(
    local = local,
    remote = remote,
    slim = slim,
    enableOpentelemetry = enableOpentelemetry,
    enableMls = enableMls,
    sharedSecret = sharedSecret,
    jwt = jwt,
    spireTrustBundle = spireTrustBundle,
    audience = audience,
    spireSocketPath = spireSocketPath,
    spireTargetSpiffeId = spireTargetSpiffeId,
    spireJwtAudience = spireJwtAudience
)

/**
 * Configuration for SLIM server.
 */
class ServerConfig(
    val slim: String = System.getenv("SLIM_ENDPOINT") ?: "127.0.0.1:46357",
    val enableOpentelemetry: Boolean = System.getenv("SLIM_ENABLE_OPENTELEMETRY")?.toBoolean() ?: false
)

/**
 * Parse command-line arguments into a configuration object.
 */
object ConfigParser {
    /**
     * Parse base configuration arguments.
     */
    fun parseBaseArgs(args: Array<String>): MutableMap<String, Any?> {
        val config = mutableMapOf<String, Any?>()
        var i = 0
        
        while (i < args.size) {
            when (args[i]) {
                "--local" -> {
                    config["local"] = args.getOrNull(++i)
                }
                "--remote" -> {
                    config["remote"] = args.getOrNull(++i)
                }
                "--slim" -> {
                    config["slim"] = args.getOrNull(++i)
                }
                "--shared-secret" -> {
                    config["sharedSecret"] = args.getOrNull(++i)
                }
                "--enable-opentelemetry" -> {
                    config["enableOpentelemetry"] = true
                }
                "--enable-mls" -> {
                    config["enableMls"] = true
                }
                "--jwt" -> {
                    config["jwt"] = args.getOrNull(++i)
                }
                "--spire-trust-bundle" -> {
                    config["spireTrustBundle"] = args.getOrNull(++i)
                }
                "--audience" -> {
                    config["audience"] = args.getOrNull(++i)?.split(",")?.map { it.trim() }
                }
                "--spire-socket-path" -> {
                    config["spireSocketPath"] = args.getOrNull(++i)
                }
                "--spire-target-spiffe-id" -> {
                    config["spireTargetSpiffeId"] = args.getOrNull(++i)
                }
                "--spire-jwt-audience" -> {
                    val audiences = config["spireJwtAudience"] as? MutableList<String> ?: mutableListOf()
                    args.getOrNull(++i)?.let { audiences.add(it) }
                    config["spireJwtAudience"] = audiences
                }
            }
            i++
        }
        
        return config
    }
    
    /**
     * Parse point-to-point specific arguments.
     */
    fun parsePointToPointArgs(args: Array<String>): PointToPointConfig {
        val baseConfig = parseBaseArgs(args)
        
        var i = 0
        while (i < args.size) {
            when (args[i]) {
                "--message" -> {
                    baseConfig["message"] = args.getOrNull(++i)
                }
                "--iterations" -> {
                    baseConfig["iterations"] = args.getOrNull(++i)?.toIntOrNull() ?: 10
                }
            }
            i++
        }
        
        return PointToPointConfig(
            local = baseConfig["local"] as? String ?: throw IllegalArgumentException("--local is required"),
            remote = baseConfig["remote"] as? String,
            slim = baseConfig["slim"] as? String ?: "http://127.0.0.1:46357",
            enableOpentelemetry = baseConfig["enableOpentelemetry"] as? Boolean ?: false,
            enableMls = baseConfig["enableMls"] as? Boolean ?: false,
            sharedSecret = baseConfig["sharedSecret"] as? String ?: "abcde-12345-fedcb-67890-deadc",
            jwt = baseConfig["jwt"] as? String,
            spireTrustBundle = baseConfig["spireTrustBundle"] as? String,
            audience = baseConfig["audience"] as? List<String>,
            spireSocketPath = baseConfig["spireSocketPath"] as? String,
            spireTargetSpiffeId = baseConfig["spireTargetSpiffeId"] as? String,
            spireJwtAudience = baseConfig["spireJwtAudience"] as? List<String>,
            message = baseConfig["message"] as? String,
            iterations = baseConfig["iterations"] as? Int ?: 10
        )
    }
    
    /**
     * Parse group specific arguments.
     */
    fun parseGroupArgs(args: Array<String>): GroupConfig {
        val baseConfig = parseBaseArgs(args)
        val invites = mutableListOf<String>()
        
        var i = 0
        while (i < args.size) {
            when (args[i]) {
                "--invites" -> {
                    args.getOrNull(++i)?.let { invites.add(it) }
                }
            }
            i++
        }
        
        return GroupConfig(
            local = baseConfig["local"] as? String ?: throw IllegalArgumentException("--local is required"),
            remote = baseConfig["remote"] as? String,
            slim = baseConfig["slim"] as? String ?: "http://127.0.0.1:46357",
            enableOpentelemetry = baseConfig["enableOpentelemetry"] as? Boolean ?: false,
            enableMls = baseConfig["enableMls"] as? Boolean ?: false,
            sharedSecret = baseConfig["sharedSecret"] as? String ?: "abcde-12345-fedcb-67890-deadc",
            jwt = baseConfig["jwt"] as? String,
            spireTrustBundle = baseConfig["spireTrustBundle"] as? String,
            audience = baseConfig["audience"] as? List<String>,
            spireSocketPath = baseConfig["spireSocketPath"] as? String,
            spireTargetSpiffeId = baseConfig["spireTargetSpiffeId"] as? String,
            spireJwtAudience = baseConfig["spireJwtAudience"] as? List<String>,
            invites = if (invites.isNotEmpty()) invites else null
        )
    }
    
    /**
     * Parse server specific arguments.
     */
    fun parseServerArgs(args: Array<String>): ServerConfig {
        var slim = "127.0.0.1:46357"
        var enableOpentelemetry = false
        
        var i = 0
        while (i < args.size) {
            when (args[i]) {
                "--slim", "-s" -> {
                    slim = args.getOrNull(++i) ?: slim
                }
                "--enable-opentelemetry", "-t" -> {
                    enableOpentelemetry = true
                }
            }
            i++
        }
        
        return ServerConfig(slim = slim, enableOpentelemetry = enableOpentelemetry)
    }
}
