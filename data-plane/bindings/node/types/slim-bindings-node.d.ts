import { type UniffiByteArray, RustBuffer, UniffiAbstractObject, UniffiRustArcPtr, UnsafeMutableRawPointer, uniffiTypeNameSymbol, destructorGuardSymbol, pointerLiteralSymbol } from 'uniffi-bindgen-react-native';
/**
 * Basic authentication configuration
 */
export type BasicAuth = {
    username: string;
    password: string;
};
export declare const BasicAuth: Readonly<{
    /**
     * Create a frozen instance of {@link BasicAuth}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<BasicAuth> & Required<Omit<BasicAuth, never>>) => BasicAuth;
    /**
     * Create a frozen instance of {@link BasicAuth}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<BasicAuth> & Required<Omit<BasicAuth, never>>) => BasicAuth;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<BasicAuth>;
}>;
/**
 * Build information for the SLIM bindings
 */
export type BuildInfo = {
    /**
     * Semantic version (e.g., "0.7.0")
     */ version: string;
    /**
     * Git commit hash (short)
     */ gitSha: string;
    /**
     * Build date in ISO 8601 UTC format
     */ buildDate: string;
    /**
     * Build profile (debug/release)
     */ profile: string;
};
export declare const BuildInfo: Readonly<{
    /**
     * Create a frozen instance of {@link BuildInfo}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<BuildInfo> & Required<Omit<BuildInfo, never>>) => BuildInfo;
    /**
     * Create a frozen instance of {@link BuildInfo}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<BuildInfo> & Required<Omit<BuildInfo, never>>) => BuildInfo;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<BuildInfo>;
}>;
/**
 * Client configuration for connecting to a SLIM server
 */
export type ClientConfig = {
    /**
     * The target endpoint the client will connect to
     */ endpoint: string;
    /**
     * Origin (HTTP Host authority override) for the client
     */ origin?: string;
    /**
     * Optional TLS SNI server name override
     */ serverName?: string;
    /**
     * Compression type
     */ compression?: CompressionType;
    /**
     * Rate limit string (e.g., "100/s" for 100 requests per second)
     */ rateLimit?: string;
    /**
     * TLS client configuration
     */ tls: TlsClientConfig;
    /**
     * Keepalive parameters
     */ keepalive?: KeepaliveConfig;
    /**
     * HTTP Proxy configuration
     */ proxy: ProxyConfig;
    /**
     * Connection timeout
     */ connectTimeout: number;
    /**
     * Request timeout
     */ requestTimeout: number;
    /**
     * Read buffer size in bytes
     */ bufferSize?: bigint;
    /**
     * Headers associated with gRPC requests
     */ headers: Map<string, string>;
    /**
     * Authentication configuration for outgoing RPCs
     */ auth: ClientAuthenticationConfig;
    /**
     * Backoff retry configuration
     */ backoff: BackoffConfig;
    /**
     * Arbitrary user-provided metadata as JSON string
     */ metadata?: string;
};
export declare const ClientConfig: Readonly<{
    /**
     * Create a frozen instance of {@link ClientConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<ClientConfig> & Required<Omit<ClientConfig, never>>) => ClientConfig;
    /**
     * Create a frozen instance of {@link ClientConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<ClientConfig> & Required<Omit<ClientConfig, never>>) => ClientConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<ClientConfig>;
}>;
/**
 * JWT authentication configuration for client-side signing
 */
export type ClientJwtAuth = {
    /**
     * JWT key configuration (encoding key for signing)
     */ key: JwtKeyType;
    /**
     * JWT audience claims to include
     */ audience?: Array<string>;
    /**
     * JWT issuer to include
     */ issuer?: string;
    /**
     * JWT subject to include
     */ subject?: string;
    /**
     * Token validity duration (default: 3600 seconds)
     */ duration: number;
};
export declare const ClientJwtAuth: Readonly<{
    /**
     * Create a frozen instance of {@link ClientJwtAuth}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<ClientJwtAuth> & Required<Omit<ClientJwtAuth, never>>) => ClientJwtAuth;
    /**
     * Create a frozen instance of {@link ClientJwtAuth}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<ClientJwtAuth> & Required<Omit<ClientJwtAuth, never>>) => ClientJwtAuth;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<ClientJwtAuth>;
}>;
/**
 * DataPlane configuration wrapper for uniffi bindings
 */
export type DataplaneConfig = {
    /**
     * DataPlane GRPC server settings
     */ servers: Array<ServerConfig>;
    /**
     * DataPlane client configs
     */ clients: Array<ClientConfig>;
};
export declare const DataplaneConfig: Readonly<{
    /**
     * Create a frozen instance of {@link DataplaneConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<DataplaneConfig> & Required<Omit<DataplaneConfig, never>>) => DataplaneConfig;
    /**
     * Create a frozen instance of {@link DataplaneConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<DataplaneConfig> & Required<Omit<DataplaneConfig, never>>) => DataplaneConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<DataplaneConfig>;
}>;
/**
 * Exponential backoff configuration
 */
export type ExponentialBackoff = {
    /**
     * Base delay
     */ base: number;
    /**
     * Multiplication factor for each retry
     */ factor: bigint;
    /**
     * Maximum delay
     */ maxDelay: number;
    /**
     * Maximum number of retry attempts
     */ maxAttempts: bigint;
    /**
     * Whether to add random jitter to delays
     */ jitter: boolean;
};
export declare const ExponentialBackoff: Readonly<{
    /**
     * Create a frozen instance of {@link ExponentialBackoff}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<ExponentialBackoff> & Required<Omit<ExponentialBackoff, never>>) => ExponentialBackoff;
    /**
     * Create a frozen instance of {@link ExponentialBackoff}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<ExponentialBackoff> & Required<Omit<ExponentialBackoff, never>>) => ExponentialBackoff;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<ExponentialBackoff>;
}>;
/**
 * Fixed interval backoff configuration
 */
export type FixedIntervalBackoff = {
    /**
     * Fixed interval between retries
     */ interval: number;
    /**
     * Maximum number of retry attempts
     */ maxAttempts: bigint;
};
export declare const FixedIntervalBackoff: Readonly<{
    /**
     * Create a frozen instance of {@link FixedIntervalBackoff}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<FixedIntervalBackoff> & Required<Omit<FixedIntervalBackoff, never>>) => FixedIntervalBackoff;
    /**
     * Create a frozen instance of {@link FixedIntervalBackoff}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<FixedIntervalBackoff> & Required<Omit<FixedIntervalBackoff, never>>) => FixedIntervalBackoff;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<FixedIntervalBackoff>;
}>;
/**
 * JWT authentication configuration for server-side verification
 */
export type JwtAuth = {
    /**
     * JWT key configuration (decoding key for verification)
     */ key: JwtKeyType;
    /**
     * JWT audience claims to verify
     */ audience?: Array<string>;
    /**
     * JWT issuer to verify
     */ issuer?: string;
    /**
     * JWT subject to verify
     */ subject?: string;
    /**
     * Token validity duration (default: 3600 seconds)
     */ duration: number;
};
export declare const JwtAuth: Readonly<{
    /**
     * Create a frozen instance of {@link JwtAuth}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<JwtAuth> & Required<Omit<JwtAuth, never>>) => JwtAuth;
    /**
     * Create a frozen instance of {@link JwtAuth}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<JwtAuth> & Required<Omit<JwtAuth, never>>) => JwtAuth;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<JwtAuth>;
}>;
/**
 * JWT key configuration
 */
export type JwtKeyConfig = {
    /**
     * Algorithm used for signing/verifying the JWT
     */ algorithm: JwtAlgorithm;
    /**
     * Key format - PEM, JWK or JWKS
     */ format: JwtKeyFormat;
    /**
     * Encoded key or file path
     */ key: JwtKeyData;
};
export declare const JwtKeyConfig: Readonly<{
    /**
     * Create a frozen instance of {@link JwtKeyConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<JwtKeyConfig> & Required<Omit<JwtKeyConfig, never>>) => JwtKeyConfig;
    /**
     * Create a frozen instance of {@link JwtKeyConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<JwtKeyConfig> & Required<Omit<JwtKeyConfig, never>>) => JwtKeyConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<JwtKeyConfig>;
}>;
/**
 * Keepalive configuration for the client
 */
export type KeepaliveConfig = {
    /**
     * TCP keepalive duration
     */ tcpKeepalive: number;
    /**
     * HTTP2 keepalive duration
     */ http2Keepalive: number;
    /**
     * Keepalive timeout
     */ timeout: number;
    /**
     * Whether to permit keepalive without an active stream
     */ keepAliveWhileIdle: boolean;
};
export declare const KeepaliveConfig: Readonly<{
    /**
     * Create a frozen instance of {@link KeepaliveConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<KeepaliveConfig> & Required<Omit<KeepaliveConfig, never>>) => KeepaliveConfig;
    /**
     * Create a frozen instance of {@link KeepaliveConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<KeepaliveConfig> & Required<Omit<KeepaliveConfig, never>>) => KeepaliveConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<KeepaliveConfig>;
}>;
/**
 * Keepalive configuration for the server
 */
export type KeepaliveServerParameters = {
    /**
     * Max connection idle time (time after which an idle connection is closed)
     */ maxConnectionIdle: number;
    /**
     * Max connection age (maximum time a connection may exist before being closed)
     */ maxConnectionAge: number;
    /**
     * Max connection age grace (additional time after max_connection_age before closing)
     */ maxConnectionAgeGrace: number;
    /**
     * Keepalive ping frequency
     */ time: number;
    /**
     * Keepalive ping timeout (time to wait for ack)
     */ timeout: number;
};
export declare const KeepaliveServerParameters: Readonly<{
    /**
     * Create a frozen instance of {@link KeepaliveServerParameters}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<KeepaliveServerParameters> & Required<Omit<KeepaliveServerParameters, never>>) => KeepaliveServerParameters;
    /**
     * Create a frozen instance of {@link KeepaliveServerParameters}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<KeepaliveServerParameters> & Required<Omit<KeepaliveServerParameters, never>>) => KeepaliveServerParameters;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<KeepaliveServerParameters>;
}>;
/**
 * Generic message context for language bindings (UniFFI-compatible)
 *
 * Provides routing and descriptive metadata needed for replying,
 * auditing, and instrumentation across different language bindings.
 * This type is exported to foreign languages via UniFFI.
 */
export type MessageContext = {
    /**
     * Fully-qualified sender identity
     */ sourceName: Name;
    /**
     * Fully-qualified destination identity (may be empty for broadcast/group scenarios)
     */ destinationName?: Name;
    /**
     * Logical/semantic type (defaults to "msg" if unspecified)
     */ payloadType: string;
    /**
     * Arbitrary key/value pairs supplied by the sender (e.g. tracing IDs)
     */ metadata: Map<string, string>;
    /**
     * Numeric identifier of the inbound connection carrying the message
     */ inputConnection: bigint;
    /**
     * Identity contained in the message
     */ identity: string;
};
export declare const MessageContext: Readonly<{
    /**
     * Create a frozen instance of {@link MessageContext}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<MessageContext> & Required<Omit<MessageContext, never>>) => MessageContext;
    /**
     * Create a frozen instance of {@link MessageContext}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<MessageContext> & Required<Omit<MessageContext, never>>) => MessageContext;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<MessageContext>;
}>;
/**
 * HTTP Proxy configuration
 */
export type ProxyConfig = {
    /**
     * The HTTP proxy URL (e.g., "http://proxy.example.com:8080")
     */ url?: string;
    /**
     * TLS configuration for proxy connection
     */ tls: TlsClientConfig;
    /**
     * Optional username for proxy authentication
     */ username?: string;
    /**
     * Optional password for proxy authentication
     */ password?: string;
    /**
     * Headers to send with proxy requests
     */ headers: Map<string, string>;
};
export declare const ProxyConfig: Readonly<{
    /**
     * Create a frozen instance of {@link ProxyConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<ProxyConfig> & Required<Omit<ProxyConfig, never>>) => ProxyConfig;
    /**
     * Create a frozen instance of {@link ProxyConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<ProxyConfig> & Required<Omit<ProxyConfig, never>>) => ProxyConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<ProxyConfig>;
}>;
/**
 * Received message containing context and payload
 */
export type ReceivedMessage = {
    context: MessageContext;
    payload: ArrayBuffer;
};
export declare const ReceivedMessage: Readonly<{
    /**
     * Create a frozen instance of {@link ReceivedMessage}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<ReceivedMessage> & Required<Omit<ReceivedMessage, never>>) => ReceivedMessage;
    /**
     * Create a frozen instance of {@link ReceivedMessage}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<ReceivedMessage> & Required<Omit<ReceivedMessage, never>>) => ReceivedMessage;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<ReceivedMessage>;
}>;
/**
 * Runtime configuration for the SLIM bindings
 *
 * Controls the Tokio runtime behavior including thread count, naming, and shutdown timeout.
 */
export type RuntimeConfig = {
    /**
     * Number of cores to use for the runtime (0 = use all available cores)
     */ nCores: bigint;
    /**
     * Thread name prefix for the runtime
     */ threadName: string;
    /**
     * Timeout duration for draining services during shutdown
     */ drainTimeout: number;
};
export declare const RuntimeConfig: Readonly<{
    /**
     * Create a frozen instance of {@link RuntimeConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<RuntimeConfig> & Required<Omit<RuntimeConfig, never>>) => RuntimeConfig;
    /**
     * Create a frozen instance of {@link RuntimeConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<RuntimeConfig> & Required<Omit<RuntimeConfig, never>>) => RuntimeConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<RuntimeConfig>;
}>;
/**
 * Server configuration for running a SLIM server
 */
export type ServerConfig = {
    /**
     * Endpoint address to listen on (e.g., "0.0.0.0:50051" or "[::]:50051")
     */ endpoint: string;
    /**
     * TLS server configuration
     */ tls: TlsServerConfig;
    /**
     * Use HTTP/2 only (default: true)
     */ http2Only: boolean;
    /**
     * Maximum size (in MiB) of messages accepted by the server
     */ maxFrameSize?: number;
    /**
     * Maximum number of concurrent streams per connection
     */ maxConcurrentStreams?: number;
    /**
     * Maximum header list size in bytes
     */ maxHeaderListSize?: number;
    /**
     * Read buffer size in bytes
     */ readBufferSize?: bigint;
    /**
     * Write buffer size in bytes
     */ writeBufferSize?: bigint;
    /**
     * Keepalive parameters
     */ keepalive: KeepaliveServerParameters;
    /**
     * Authentication configuration for incoming requests
     */ auth: ServerAuthenticationConfig;
    /**
     * Arbitrary user-provided metadata as JSON string
     */ metadata?: string;
};
export declare const ServerConfig: Readonly<{
    /**
     * Create a frozen instance of {@link ServerConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<ServerConfig> & Required<Omit<ServerConfig, never>>) => ServerConfig;
    /**
     * Create a frozen instance of {@link ServerConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<ServerConfig> & Required<Omit<ServerConfig, never>>) => ServerConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<ServerConfig>;
}>;
/**
 * Service configuration wrapper for uniffi bindings
 */
export type ServiceConfig = {
    /**
     * Optional node ID for the service
     */ nodeId?: string;
    /**
     * Optional group name for the service
     */ groupName?: string;
    /**
     * DataPlane configuration (servers and clients)
     */ dataplane: DataplaneConfig;
};
export declare const ServiceConfig: Readonly<{
    /**
     * Create a frozen instance of {@link ServiceConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<ServiceConfig> & Required<Omit<ServiceConfig, never>>) => ServiceConfig;
    /**
     * Create a frozen instance of {@link ServiceConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<ServiceConfig> & Required<Omit<ServiceConfig, never>>) => ServiceConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<ServiceConfig>;
}>;
/**
 * Session configuration
 */
export type SessionConfig = {
    /**
     * Session type (PointToPoint or Group)
     */ sessionType: SessionType;
    /**
     * Enable MLS encryption for this session
     */ enableMls: boolean;
    /**
     * Maximum number of retries for message transmission (None = use default)
     */ maxRetries?: number;
    /**
     * Interval between retries in milliseconds (None = use default)
     */ interval?: number;
    /**
     * Custom metadata key-value pairs for the session
     */ metadata: Map<string, string>;
};
export declare const SessionConfig: Readonly<{
    /**
     * Create a frozen instance of {@link SessionConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<SessionConfig> & Required<Omit<SessionConfig, never>>) => SessionConfig;
    /**
     * Create a frozen instance of {@link SessionConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<SessionConfig> & Required<Omit<SessionConfig, never>>) => SessionConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<SessionConfig>;
}>;
/**
 * Result of creating a session, containing the session context and a completion handle
 *
 * The completion handle should be awaited to ensure the session is fully established.
 */
export type SessionWithCompletion = {
    /**
     * The session context for performing operations
     */ session: Session;
    /**
     * Completion handle to wait for session establishment
     */ completion: CompletionHandle;
};
export declare const SessionWithCompletion: Readonly<{
    /**
     * Create a frozen instance of {@link SessionWithCompletion}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<SessionWithCompletion> & Required<Omit<SessionWithCompletion, never>>) => SessionWithCompletion;
    /**
     * Create a frozen instance of {@link SessionWithCompletion}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<SessionWithCompletion> & Required<Omit<SessionWithCompletion, never>>) => SessionWithCompletion;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<SessionWithCompletion>;
}>;
/**
 * SPIRE configuration for SPIFFE Workload API integration
 */
export type SpireConfig = {
    /**
     * Path to the SPIFFE Workload API socket (None => use SPIFFE_ENDPOINT_SOCKET env var)
     */ socketPath?: string;
    /**
     * Optional target SPIFFE ID when requesting JWT SVIDs
     */ targetSpiffeId?: string;
    /**
     * Audiences to request/verify for JWT SVIDs
     */ jwtAudiences: Array<string>;
    /**
     * Optional trust domains override for X.509 bundle retrieval
     */ trustDomains: Array<string>;
};
export declare const SpireConfig: Readonly<{
    /**
     * Create a frozen instance of {@link SpireConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<SpireConfig> & Required<Omit<SpireConfig, never>>) => SpireConfig;
    /**
     * Create a frozen instance of {@link SpireConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<SpireConfig> & Required<Omit<SpireConfig, never>>) => SpireConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<SpireConfig>;
}>;
/**
 * Static JWT (Bearer token) authentication configuration
 * The token is loaded from a file and automatically reloaded when changed
 */
export type StaticJwtAuth = {
    /**
     * Path to file containing the JWT token
     */ tokenFile: string;
    /**
     * Duration for caching the token before re-reading from file (default: 3600 seconds)
     */ duration: number;
};
export declare const StaticJwtAuth: Readonly<{
    /**
     * Create a frozen instance of {@link StaticJwtAuth}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<StaticJwtAuth> & Required<Omit<StaticJwtAuth, never>>) => StaticJwtAuth;
    /**
     * Create a frozen instance of {@link StaticJwtAuth}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<StaticJwtAuth> & Required<Omit<StaticJwtAuth, never>>) => StaticJwtAuth;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<StaticJwtAuth>;
}>;
/**
 * TLS configuration for client connections
 */
export type TlsClientConfig = {
    /**
     * Disable TLS entirely (plain text connection)
     */ insecure: boolean;
    /**
     * Skip server certificate verification (enables TLS but doesn't verify certs)
     * WARNING: Only use for testing - insecure in production!
     */ insecureSkipVerify: boolean;
    /**
     * Certificate and key source for client authentication
     */ source: TlsSource;
    /**
     * CA certificate source for verifying server certificates
     */ caSource: CaSource;
    /**
     * Include system CA certificates pool (default: true)
     */ includeSystemCaCertsPool: boolean;
    /**
     * TLS version to use: "tls1.2" or "tls1.3" (default: "tls1.3")
     */ tlsVersion: string;
};
export declare const TlsClientConfig: Readonly<{
    /**
     * Create a frozen instance of {@link TlsClientConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<TlsClientConfig> & Required<Omit<TlsClientConfig, never>>) => TlsClientConfig;
    /**
     * Create a frozen instance of {@link TlsClientConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<TlsClientConfig> & Required<Omit<TlsClientConfig, never>>) => TlsClientConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<TlsClientConfig>;
}>;
/**
 * TLS configuration for server connections
 */
export type TlsServerConfig = {
    /**
     * Disable TLS entirely (plain text connection)
     */ insecure: boolean;
    /**
     * Certificate and key source for server authentication
     */ source: TlsSource;
    /**
     * CA certificate source for verifying client certificates
     */ clientCa: CaSource;
    /**
     * Include system CA certificates pool (default: true)
     */ includeSystemCaCertsPool: boolean;
    /**
     * TLS version to use: "tls1.2" or "tls1.3" (default: "tls1.3")
     */ tlsVersion: string;
    /**
     * Reload client CA file when modified
     */ reloadClientCaFile: boolean;
};
export declare const TlsServerConfig: Readonly<{
    /**
     * Create a frozen instance of {@link TlsServerConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<TlsServerConfig> & Required<Omit<TlsServerConfig, never>>) => TlsServerConfig;
    /**
     * Create a frozen instance of {@link TlsServerConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<TlsServerConfig> & Required<Omit<TlsServerConfig, never>>) => TlsServerConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<TlsServerConfig>;
}>;
/**
 * Tracing/logging configuration for the SLIM bindings
 *
 * Controls logging behavior including log level, thread name/ID display, and filters.
 */
export type TracingConfig = {
    /**
     * Log level (e.g., "debug", "info", "warn", "error")
     */ logLevel: string;
    /**
     * Whether to display thread names in logs
     */ displayThreadNames: boolean;
    /**
     * Whether to display thread IDs in logs
     */ displayThreadIds: boolean;
    /**
     * List of tracing filter directives (e.g., ["slim=debug", "tokio=info"])
     */ filters: Array<string>;
};
export declare const TracingConfig: Readonly<{
    /**
     * Create a frozen instance of {@link TracingConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    create: (partial: Partial<TracingConfig> & Required<Omit<TracingConfig, never>>) => TracingConfig;
    /**
     * Create a frozen instance of {@link TracingConfig}, with defaults specified
     * in Rust, in the {@link slim_bindings} crate.
     */
    new: (partial: Partial<TracingConfig> & Required<Omit<TracingConfig, never>>) => TracingConfig;
    /**
     * Defaults specified in the {@link slim_bindings} crate.
     */
    defaults: () => Partial<TracingConfig>;
}>;
/**
 * Backoff retry configuration
 */
export type BackoffConfig = {
    tag: "exponential";
    inner: Readonly<{
        config: ExponentialBackoff;
    }>;
} | {
    tag: "fixedInterval";
    inner: Readonly<{
        config: FixedIntervalBackoff;
    }>;
};
export declare const FfiConverterTypeBackoffConfig: {
    read(from: RustBuffer): BackoffConfig;
    write(value: BackoffConfig, into: RustBuffer): void;
    allocationSize(value: BackoffConfig): number;
    lift(value: UniffiByteArray): BackoffConfig;
    lower(value: BackoffConfig): UniffiByteArray;
};
/**
 * CA certificate source configuration
 */
export type CaSource = 
/**
 * Load CA from file
 */
{
    tag: "file";
    inner: Readonly<{
        path: string;
    }>;
}
/**
 * Load CA from PEM string
 */
 | {
    tag: "pem";
    inner: Readonly<{
        data: string;
    }>;
}
/**
 * Load CA from SPIRE Workload API
 */
 | {
    tag: "spire";
    inner: Readonly<{
        config: SpireConfig;
    }>;
}
/**
 * No CA configured
 */
 | {
    tag: "none";
};
export declare const FfiConverterTypeCaSource: {
    read(from: RustBuffer): CaSource;
    write(value: CaSource, into: RustBuffer): void;
    allocationSize(value: CaSource): number;
    lift(value: UniffiByteArray): CaSource;
    lower(value: CaSource): UniffiByteArray;
};
/**
 * Authentication configuration enum for client
 */
export type ClientAuthenticationConfig = {
    tag: "basic";
    inner: Readonly<{
        config: BasicAuth;
    }>;
} | {
    tag: "staticJwt";
    inner: Readonly<{
        config: StaticJwtAuth;
    }>;
} | {
    tag: "jwt";
    inner: Readonly<{
        config: ClientJwtAuth;
    }>;
} | {
    tag: "none";
};
export declare const FfiConverterTypeClientAuthenticationConfig: {
    read(from: RustBuffer): ClientAuthenticationConfig;
    write(value: ClientAuthenticationConfig, into: RustBuffer): void;
    allocationSize(value: ClientAuthenticationConfig): number;
    lift(value: UniffiByteArray): ClientAuthenticationConfig;
    lower(value: ClientAuthenticationConfig): UniffiByteArray;
};
/**
 * Compression type for gRPC messages
 */
export type CompressionType = "gzip" | "zlib" | "deflate" | "snappy" | "zstd" | "lz4" | "none" | "empty";
export declare const FfiConverterTypeCompressionType: {
    read(from: RustBuffer): CompressionType;
    write(value: CompressionType, into: RustBuffer): void;
    allocationSize(value: CompressionType): number;
    lift(value: UniffiByteArray): CompressionType;
    lower(value: CompressionType): UniffiByteArray;
};
/**
 * Direction enum
 * Indicates whether the App can send, receive, both, or neither.
 */
export type Direction = "send" | "recv" | "bidirectional" | "none";
export declare const FfiConverterTypeDirection: {
    read(from: RustBuffer): Direction;
    write(value: Direction, into: RustBuffer): void;
    allocationSize(value: Direction): number;
    lift(value: UniffiByteArray): Direction;
    lower(value: Direction): UniffiByteArray;
};
/**
 * Identity provider configuration - used to prove identity to others
 */
export type IdentityProviderConfig = 
/**
 * Shared secret authentication (symmetric key)
 */
{
    tag: "sharedSecret";
    inner: Readonly<{
        id: string;
        data: string;
    }>;
}
/**
 * Static JWT loaded from file with auto-reload
 */
 | {
    tag: "staticJwt";
    inner: Readonly<{
        config: StaticJwtAuth;
    }>;
}
/**
 * Dynamic JWT generation with signing key
 */
 | {
    tag: "jwt";
    inner: Readonly<{
        config: ClientJwtAuth;
    }>;
}
/**
 * SPIRE-based identity provider (non-Windows only)
 */
 | {
    tag: "spire";
    inner: Readonly<{
        config: SpireConfig;
    }>;
}
/**
 * No identity provider configured
 */
 | {
    tag: "none";
};
export declare const FfiConverterTypeIdentityProviderConfig: {
    read(from: RustBuffer): IdentityProviderConfig;
    write(value: IdentityProviderConfig, into: RustBuffer): void;
    allocationSize(value: IdentityProviderConfig): number;
    lift(value: UniffiByteArray): IdentityProviderConfig;
    lower(value: IdentityProviderConfig): UniffiByteArray;
};
/**
 * Identity verifier configuration - used to verify identity of others
 */
export type IdentityVerifierConfig = 
/**
 * Shared secret verification (symmetric key)
 */
{
    tag: "sharedSecret";
    inner: Readonly<{
        id: string;
        data: string;
    }>;
}
/**
 * JWT verification with decoding key
 */
 | {
    tag: "jwt";
    inner: Readonly<{
        config: JwtAuth;
    }>;
}
/**
 * SPIRE-based identity verifier (non-Windows only)
 */
 | {
    tag: "spire";
    inner: Readonly<{
        config: SpireConfig;
    }>;
}
/**
 * No identity verifier configured
 */
 | {
    tag: "none";
};
export declare const FfiConverterTypeIdentityVerifierConfig: {
    read(from: RustBuffer): IdentityVerifierConfig;
    write(value: IdentityVerifierConfig, into: RustBuffer): void;
    allocationSize(value: IdentityVerifierConfig): number;
    lift(value: UniffiByteArray): IdentityVerifierConfig;
    lower(value: IdentityVerifierConfig): UniffiByteArray;
};
/**
 * JWT signing/verification algorithm
 */
export type JwtAlgorithm = "hs256" | "hs384" | "hs512" | "es256" | "es384" | "rs256" | "rs384" | "rs512" | "ps256" | "ps384" | "ps512" | "edDsa";
export declare const FfiConverterTypeJwtAlgorithm: {
    read(from: RustBuffer): JwtAlgorithm;
    write(value: JwtAlgorithm, into: RustBuffer): void;
    allocationSize(value: JwtAlgorithm): number;
    lift(value: UniffiByteArray): JwtAlgorithm;
    lower(value: JwtAlgorithm): UniffiByteArray;
};
/**
 * JWT key data source
 */
export type JwtKeyData = 
/**
 * String with encoded key(s)
 */
{
    tag: "data";
    inner: Readonly<{
        value: string;
    }>;
}
/**
 * File path to the key(s)
 */
 | {
    tag: "file";
    inner: Readonly<{
        path: string;
    }>;
};
export declare const FfiConverterTypeJwtKeyData: {
    read(from: RustBuffer): JwtKeyData;
    write(value: JwtKeyData, into: RustBuffer): void;
    allocationSize(value: JwtKeyData): number;
    lift(value: UniffiByteArray): JwtKeyData;
    lower(value: JwtKeyData): UniffiByteArray;
};
/**
 * JWT key format
 */
export type JwtKeyFormat = "pem" | "jwk" | "jwks";
export declare const FfiConverterTypeJwtKeyFormat: {
    read(from: RustBuffer): JwtKeyFormat;
    write(value: JwtKeyFormat, into: RustBuffer): void;
    allocationSize(value: JwtKeyFormat): number;
    lift(value: UniffiByteArray): JwtKeyFormat;
    lower(value: JwtKeyFormat): UniffiByteArray;
};
/**
 * JWT key type (encoding, decoding, or autoresolve)
 */
export type JwtKeyType = 
/**
 * Encoding key for signing JWTs (client-side)
 */
{
    tag: "encoding";
    inner: Readonly<{
        key: JwtKeyConfig;
    }>;
}
/**
 * Decoding key for verifying JWTs (server-side)
 */
 | {
    tag: "decoding";
    inner: Readonly<{
        key: JwtKeyConfig;
    }>;
}
/**
 * Automatically resolve keys based on claims
 */
 | {
    tag: "autoresolve";
};
export declare const FfiConverterTypeJwtKeyType: {
    read(from: RustBuffer): JwtKeyType;
    write(value: JwtKeyType, into: RustBuffer): void;
    allocationSize(value: JwtKeyType): number;
    lift(value: UniffiByteArray): JwtKeyType;
    lower(value: JwtKeyType): UniffiByteArray;
};
/**
 * Authentication configuration enum for server
 */
export type ServerAuthenticationConfig = {
    tag: "basic";
    inner: Readonly<{
        config: BasicAuth;
    }>;
} | {
    tag: "jwt";
    inner: Readonly<{
        config: JwtAuth;
    }>;
} | {
    tag: "none";
};
export declare const FfiConverterTypeServerAuthenticationConfig: {
    read(from: RustBuffer): ServerAuthenticationConfig;
    write(value: ServerAuthenticationConfig, into: RustBuffer): void;
    allocationSize(value: ServerAuthenticationConfig): number;
    lift(value: UniffiByteArray): ServerAuthenticationConfig;
    lower(value: ServerAuthenticationConfig): UniffiByteArray;
};
/**
 * Session type enum
 */
export type SessionType = "pointToPoint" | "group";
export declare const FfiConverterTypeSessionType: {
    read(from: RustBuffer): SessionType;
    write(value: SessionType, into: RustBuffer): void;
    allocationSize(value: SessionType): number;
    lift(value: UniffiByteArray): SessionType;
    lower(value: SessionType): UniffiByteArray;
};
/**
 * Error types for SLIM operations
 */
export type SlimError = {
    tag: "serviceError";
    inner: Readonly<{
        message: string;
    }>;
} | {
    tag: "sessionError";
    inner: Readonly<{
        message: string;
    }>;
} | {
    tag: "receiveError";
    inner: Readonly<{
        message: string;
    }>;
} | {
    tag: "sendError";
    inner: Readonly<{
        message: string;
    }>;
} | {
    tag: "authError";
    inner: Readonly<{
        message: string;
    }>;
} | {
    tag: "configError";
    inner: Readonly<{
        message: string;
    }>;
} | {
    tag: "timeout";
} | {
    tag: "invalidArgument";
    inner: Readonly<{
        message: string;
    }>;
} | {
    tag: "internalError";
    inner: Readonly<{
        message: string;
    }>;
};
export declare const FfiConverterTypeSlimError: {
    read(from: RustBuffer): SlimError;
    write(value: SlimError, into: RustBuffer): void;
    allocationSize(value: SlimError): number;
    lift(value: UniffiByteArray): SlimError;
    lower(value: SlimError): UniffiByteArray;
};
/**
 * TLS certificate and key source configuration
 */
export type TlsSource = 
/**
 * Load certificate and key from PEM strings
 */
{
    tag: "pem";
    inner: Readonly<{
        cert: string;
        key: string;
    }>;
}
/**
 * Load certificate and key from files (with auto-reload support)
 */
 | {
    tag: "file";
    inner: Readonly<{
        cert: string;
        key: string;
    }>;
}
/**
 * Load certificate and key from SPIRE Workload API
 */
 | {
    tag: "spire";
    inner: Readonly<{
        config: SpireConfig;
    }>;
}
/**
 * No certificate/key configured
 */
 | {
    tag: "none";
};
export declare const FfiConverterTypeTlsSource: {
    read(from: RustBuffer): TlsSource;
    write(value: TlsSource, into: RustBuffer): void;
    allocationSize(value: TlsSource): number;
    lift(value: UniffiByteArray): TlsSource;
    lower(value: TlsSource): UniffiByteArray;
};
export type AppInterface = {
    /**
     * Create a new session (blocking version for FFI)
     *
     * Returns a SessionWithCompletion containing the session context and a completion handle.
     * Call `.wait()` on the completion handle to wait for session establishment.
     */ createSession(config: SessionConfig, destination: Name): SessionWithCompletion;
    /**
     * Create a new session and wait for completion (blocking version)
     *
     * This method creates a session and blocks until the session establishment completes.
     * Returns only the session context, as the completion has already been awaited.
     */ createSessionAndWait(config: SessionConfig, destination: Name): Session;
    /**
     * Create a new session and wait for completion (async version)
     *
     * This method creates a session and waits until the session establishment completes.
     * Returns only the session context, as the completion has already been awaited.
     */ createSessionAndWaitAsync(config: SessionConfig, destination: Name, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<Session>;
    /**
     * Create a new session (async version)
     *
     * Returns a SessionWithCompletion containing the session context and a completion handle.
     * Await the completion handle to wait for session establishment.
     * For point-to-point sessions, this ensures the remote peer has acknowledged the session.
     * For multicast sessions, this ensures the initial setup is complete.
     */ createSessionAsync(config: SessionConfig, destination: Name, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<SessionWithCompletion>;
    /**
     * Delete a session (blocking version for FFI)
     *
     * Returns a completion handle that can be awaited to ensure the deletion completes.
     */ deleteSession(session: Session): CompletionHandle;
    /**
     * Delete a session and wait for completion (blocking version)
     *
     * This method deletes a session and blocks until the deletion completes.
     */ deleteSessionAndWait(session: Session): void;
    /**
     * Delete a session and wait for completion (async version)
     *
     * This method deletes a session and waits until the deletion completes.
     */ deleteSessionAndWaitAsync(session: Session, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Delete a session (async version)
     *
     * Returns a completion handle that can be awaited to ensure the deletion completes.
     */ deleteSessionAsync(session: Session, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<CompletionHandle>;
    /**
     * Get the app ID (derived from name)
     */ id(): bigint;
    /**
     * Listen for incoming sessions (blocking version for FFI)
     */ listenForSession(timeout: number | undefined): Session;
    /**
     * Listen for incoming sessions (async version)
     */ listenForSessionAsync(timeout: number | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<Session>;
    /**
     * Get the app name
     */ name(): Name;
    /**
     * Remove a route (blocking version for FFI)
     */ removeRoute(name: Name, connectionId: bigint): void;
    /**
     * Remove a route (async version)
     */ removeRouteAsync(name: Name, connectionId: bigint, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Set a route to a name for a specific connection (blocking version for FFI)
     */ setRoute(name: Name, connectionId: bigint): void;
    /**
     * Set a route to a name for a specific connection (async version)
     */ setRouteAsync(name: Name, connectionId: bigint, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Subscribe to a session name (blocking version for FFI)
     */ subscribe(name: Name, connectionId: /*u64*/ bigint | undefined): void;
    /**
     * Subscribe to a name (async version)
     */ subscribeAsync(name: Name, connectionId: /*u64*/ bigint | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Unsubscribe from a name (blocking version for FFI)
     */ unsubscribe(name: Name, connectionId: /*u64*/ bigint | undefined): void;
    /**
     * Unsubscribe from a name (async version)
     */ unsubscribeAsync(name: Name, connectionId: /*u64*/ bigint | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
};
/**
 * Adapter that bridges the App API with language-bindings interface
 *
 * This adapter uses enum-based auth types (`AuthProvider`/`AuthVerifier`) instead of generics
 * to be compatible with UniFFI, supporting multiple authentication mechanisms (SharedSecret,
 * JWT, SPIRE, StaticToken). It provides both synchronous (blocking) and asynchronous methods
 * for flexibility.
 */
export declare class App extends UniffiAbstractObject implements AppInterface {
    readonly [uniffiTypeNameSymbol] = "App";
    readonly [destructorGuardSymbol]: UniffiRustArcPtr;
    readonly [pointerLiteralSymbol]: UnsafeMutableRawPointer;
    /**
     * Create a new App with identity provider and verifier configurations
     *
     * This is the main entry point for creating a SLIM application from language bindings.
     *
     * # Arguments
     * * `base_name` - The base name for the app (without ID)
     * * `identity_provider_config` - Configuration for proving identity to others
     * * `identity_verifier_config` - Configuration for verifying identity of others
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created adapter
     * * `Err(SlimError)` - If adapter creation fails
     *
     * # Supported Identity Types
     * - SharedSecret: Symmetric key authentication
     * - JWT: Dynamic JWT generation/verification with signing/decoding keys
     * - StaticJWT: Static JWT loaded from file with auto-reload
     */
    constructor(baseName: Name, identityProviderConfig: IdentityProviderConfig, identityVerifierConfig: IdentityVerifierConfig);
    /**
     * Create a new App with traffic direction (blocking version)
     *
     * This is a convenience function for creating a SLIM application with configurable
     * traffic direction (send-only, receive-only, bidirectional, or none).
     *
     * # Arguments
     * * `name` - The base name for the app (without ID)
     * * `identity_provider_config` - Configuration for proving identity to others
     * * `identity_verifier_config` - Configuration for verifying identity of others
     * * `direction` - Traffic direction for sessions (Send, Recv, Bidirectional, or None)
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created app
     * * `Err(SlimError)` - If app creation fails
     */
    static newWithDirection(name: Name, identityProviderConfig: IdentityProviderConfig, identityVerifierConfig: IdentityVerifierConfig, direction: Direction): App;
    /**
     * Create a new App with SharedSecret authentication (blocking version)
     *
     * This is a convenience function for creating a SLIM application using SharedSecret authentication.
     *
     * # Arguments
     * * `name` - The base name for the app (without ID)
     * * `secret` - The shared secret string for authentication
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created adapter
     * * `Err(SlimError)` - If adapter creation fails
     */
    static newWithSecret(name: Name, secret: string): App;
    /**
     * Create a new session (blocking version for FFI)
     *
     * Returns a SessionWithCompletion containing the session context and a completion handle.
     * Call `.wait()` on the completion handle to wait for session establishment.
     */ createSession(config: SessionConfig, destination: Name): SessionWithCompletion;
    /**
     * Create a new session and wait for completion (blocking version)
     *
     * This method creates a session and blocks until the session establishment completes.
     * Returns only the session context, as the completion has already been awaited.
     */ createSessionAndWait(config: SessionConfig, destination: Name): Session;
    /**
     * Create a new session and wait for completion (async version)
     *
     * This method creates a session and waits until the session establishment completes.
     * Returns only the session context, as the completion has already been awaited.
     */ createSessionAndWaitAsync(config: SessionConfig, destination: Name, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<Session>;
    /**
     * Create a new session (async version)
     *
     * Returns a SessionWithCompletion containing the session context and a completion handle.
     * Await the completion handle to wait for session establishment.
     * For point-to-point sessions, this ensures the remote peer has acknowledged the session.
     * For multicast sessions, this ensures the initial setup is complete.
     */ createSessionAsync(config: SessionConfig, destination: Name, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<SessionWithCompletion>;
    /**
     * Delete a session (blocking version for FFI)
     *
     * Returns a completion handle that can be awaited to ensure the deletion completes.
     */ deleteSession(session: Session): CompletionHandle;
    /**
     * Delete a session and wait for completion (blocking version)
     *
     * This method deletes a session and blocks until the deletion completes.
     */ deleteSessionAndWait(session: Session): void;
    /**
     * Delete a session and wait for completion (async version)
     *
     * This method deletes a session and waits until the deletion completes.
     */ deleteSessionAndWaitAsync(session: Session, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Delete a session (async version)
     *
     * Returns a completion handle that can be awaited to ensure the deletion completes.
     */ deleteSessionAsync(session: Session, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<CompletionHandle>;
    /**
     * Get the app ID (derived from name)
     */ id(): bigint;
    /**
     * Listen for incoming sessions (blocking version for FFI)
     */ listenForSession(timeout: number | undefined): Session;
    /**
     * Listen for incoming sessions (async version)
     */ listenForSessionAsync(timeout: number | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<Session>;
    /**
     * Get the app name
     */ name(): Name;
    /**
     * Remove a route (blocking version for FFI)
     */ removeRoute(name: Name, connectionId: bigint): void;
    /**
     * Remove a route (async version)
     */ removeRouteAsync(name: Name, connectionId: bigint, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Set a route to a name for a specific connection (blocking version for FFI)
     */ setRoute(name: Name, connectionId: bigint): void;
    /**
     * Set a route to a name for a specific connection (async version)
     */ setRouteAsync(name: Name, connectionId: bigint, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Subscribe to a session name (blocking version for FFI)
     */ subscribe(name: Name, connectionId: /*u64*/ bigint | undefined): void;
    /**
     * Subscribe to a name (async version)
     */ subscribeAsync(name: Name, connectionId: /*u64*/ bigint | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Unsubscribe from a name (blocking version for FFI)
     */ unsubscribe(name: Name, connectionId: /*u64*/ bigint | undefined): void;
    /**
     * Unsubscribe from a name (async version)
     */ unsubscribeAsync(name: Name, connectionId: /*u64*/ bigint | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * {@inheritDoc uniffi-bindgen-react-native#UniffiAbstractObject.uniffiDestroy}
     */
    uniffiDestroy(): void;
    [Symbol.dispose]: () => void;
    static instanceOf(obj: any): obj is App;
}
export type CompletionHandleInterface = {
    /**
     * Wait for the operation to complete indefinitely (blocking version)
     *
     * This blocks the calling thread until the operation completes.
     * Use this from Go or other languages when you need to ensure
     * an operation has finished before proceeding.
     *
     * **Note:** This can only be called once per handle. Subsequent calls
     * will return an error.
     *
     * # Returns
     * * `Ok(())` - Operation completed successfully
     * * `Err(SlimError)` - Operation failed or handle already consumed
     */ wait(): void;
    /**
     * Wait for the operation to complete indefinitely (async version)
     *
     * This is the async version that integrates with UniFFI's polling mechanism.
     * The operation will yield control while waiting.
     *
     * **Note:** This can only be called once per handle. Subsequent calls
     * will return an error.
     *
     * # Returns
     * * `Ok(())` - Operation completed successfully
     * * `Err(SlimError)` - Operation failed or handle already consumed
     */ waitAsync(asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Wait for the operation to complete with a timeout (blocking version)
     *
     * This blocks the calling thread until the operation completes or the timeout expires.
     * Use this from Go or other languages when you need to ensure
     * an operation has finished before proceeding with a time limit.
     *
     * **Note:** This can only be called once per handle. Subsequent calls
     * will return an error.
     *
     * # Arguments
     * * `timeout` - Maximum time to wait for completion
     *
     * # Returns
     * * `Ok(())` - Operation completed successfully
     * * `Err(SlimError::Timeout)` - If the operation timed out
     * * `Err(SlimError)` - Operation failed or handle already consumed
     */ waitFor(timeout: number): void;
    /**
     * Wait for the operation to complete with a timeout (async version)
     *
     * This is the async version that integrates with UniFFI's polling mechanism.
     * The operation will yield control while waiting until completion or timeout.
     *
     * **Note:** This can only be called once per handle. Subsequent calls
     * will return an error.
     *
     * # Arguments
     * * `timeout` - Maximum time to wait for completion
     *
     * # Returns
     * * `Ok(())` - Operation completed successfully
     * * `Err(SlimError::Timeout)` - If the operation timed out
     * * `Err(SlimError)` - Operation failed or handle already consumed
     */ waitForAsync(timeout: number, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
};
/**
 * FFI-compatible completion handle for async operations
 *
 * Represents a pending operation that can be awaited to ensure completion.
 * Used for operations that need delivery confirmation or handshake acknowledgment.
 *
 * # Examples
 *
 * Basic usage:
 * ```ignore
 * let completion = session.publish(data, None, None)?;
 * completion.wait()?; // Wait for delivery confirmation
 * ```
 */
export declare class CompletionHandle extends UniffiAbstractObject implements CompletionHandleInterface {
    readonly [uniffiTypeNameSymbol] = "CompletionHandle";
    readonly [destructorGuardSymbol]: UniffiRustArcPtr;
    readonly [pointerLiteralSymbol]: UnsafeMutableRawPointer;
    /**
     * Wait for the operation to complete indefinitely (blocking version)
     *
     * This blocks the calling thread until the operation completes.
     * Use this from Go or other languages when you need to ensure
     * an operation has finished before proceeding.
     *
     * **Note:** This can only be called once per handle. Subsequent calls
     * will return an error.
     *
     * # Returns
     * * `Ok(())` - Operation completed successfully
     * * `Err(SlimError)` - Operation failed or handle already consumed
     */ wait(): void;
    /**
     * Wait for the operation to complete indefinitely (async version)
     *
     * This is the async version that integrates with UniFFI's polling mechanism.
     * The operation will yield control while waiting.
     *
     * **Note:** This can only be called once per handle. Subsequent calls
     * will return an error.
     *
     * # Returns
     * * `Ok(())` - Operation completed successfully
     * * `Err(SlimError)` - Operation failed or handle already consumed
     */ waitAsync(asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Wait for the operation to complete with a timeout (blocking version)
     *
     * This blocks the calling thread until the operation completes or the timeout expires.
     * Use this from Go or other languages when you need to ensure
     * an operation has finished before proceeding with a time limit.
     *
     * **Note:** This can only be called once per handle. Subsequent calls
     * will return an error.
     *
     * # Arguments
     * * `timeout` - Maximum time to wait for completion
     *
     * # Returns
     * * `Ok(())` - Operation completed successfully
     * * `Err(SlimError::Timeout)` - If the operation timed out
     * * `Err(SlimError)` - Operation failed or handle already consumed
     */ waitFor(timeout: number): void;
    /**
     * Wait for the operation to complete with a timeout (async version)
     *
     * This is the async version that integrates with UniFFI's polling mechanism.
     * The operation will yield control while waiting until completion or timeout.
     *
     * **Note:** This can only be called once per handle. Subsequent calls
     * will return an error.
     *
     * # Arguments
     * * `timeout` - Maximum time to wait for completion
     *
     * # Returns
     * * `Ok(())` - Operation completed successfully
     * * `Err(SlimError::Timeout)` - If the operation timed out
     * * `Err(SlimError)` - Operation failed or handle already consumed
     */ waitForAsync(timeout: number, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * {@inheritDoc uniffi-bindgen-react-native#UniffiAbstractObject.uniffiDestroy}
     */
    uniffiDestroy(): void;
    [Symbol.dispose]: () => void;
    static instanceOf(obj: any): obj is CompletionHandle;
}
export type NameInterface = {
    /**
     * Get the name components as a vector of strings
     */ components(): Array<string>;
    /**
     * Get the name ID
     */ id(): bigint;
};
/**
 * Name type for SLIM (Secure Low-Latency Interactive Messaging)
 */
export declare class Name extends UniffiAbstractObject implements NameInterface {
    readonly [uniffiTypeNameSymbol] = "Name";
    readonly [destructorGuardSymbol]: UniffiRustArcPtr;
    readonly [pointerLiteralSymbol]: UnsafeMutableRawPointer;
    /**
     * Create a new Name from components without an ID
     */
    constructor(component0: string, component1: string, component2: string);
    /**
     * Create a new Name from components with an ID
     */
    static newWithId(component0: string, component1: string, component2: string, id: bigint): Name;
    /**
     * Get the name components as a vector of strings
     */ components(): Array<string>;
    /**
     * Get the name ID
     */ id(): bigint;
    /**
     * {@inheritDoc uniffi-bindgen-react-native#UniffiAbstractObject.uniffiDestroy}
     */
    uniffiDestroy(): void;
    [Symbol.dispose]: () => void;
    static instanceOf(obj: any): obj is Name;
}
export type ServiceInterface = {
    /**
     * Get the service configuration
     */ config(): ServiceConfig;
    /**
     * Connect to a remote endpoint as a client - blocking version
     */ connect(config: ClientConfig): bigint;
    /**
     * Connect to a remote endpoint as a client
     */ connectAsync(config: ClientConfig, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise</*u64*/ bigint>;
    /**
     * Create a new App with authentication configuration (blocking version)
     *
     * This method initializes authentication providers/verifiers and creates a App
     * on this service instance. This is a blocking wrapper around create_app_async.
     *
     * # Arguments
     * * `base_name` - The base name for the app (without ID)
     * * `identity_provider_config` - Configuration for proving identity to others
     * * `identity_verifier_config` - Configuration for verifying identity of others
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created adapter
     * * `Err(SlimError)` - If adapter creation fails
     */ createApp(baseName: Name, identityProviderConfig: IdentityProviderConfig, identityVerifierConfig: IdentityVerifierConfig): App;
    /**
     * Create a new App with authentication configuration (async version)
     *
     * This method initializes authentication providers/verifiers and creates a App
     * on this service instance.
     *
     * # Arguments
     * * `base_name` - The base name for the app (without ID)
     * * `identity_provider_config` - Configuration for proving identity to others
     * * `identity_verifier_config` - Configuration for verifying identity of others
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created adapter
     * * `Err(SlimError)` - If adapter creation fails
     */ createAppAsync(baseName: Name, identityProviderConfig: IdentityProviderConfig, identityVerifierConfig: IdentityVerifierConfig, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<App>;
    /**
     * Create a new App with authentication configuration and traffic direction (blocking version)
     *
     * This method initializes authentication providers/verifiers and creates an App
     * on this service instance. The direction parameter controls whether the app
     * can send messages, receive messages, both, or neither.
     *
     * # Arguments
     * * `base_name` - The base name for the app (without ID)
     * * `identity_provider_config` - Configuration for proving identity to others
     * * `identity_verifier_config` - Configuration for verifying identity of others
     * * `direction` - Traffic direction: Send, Recv, Bidirectional, or None
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created adapter
     * * `Err(SlimError)` - If adapter creation fails
     */ createAppWithDirection(baseName: Name, identityProviderConfig: IdentityProviderConfig, identityVerifierConfig: IdentityVerifierConfig, direction: Direction): App;
    /**
     * Create a new App with authentication configuration and traffic direction (async version)
     *
     * This method initializes authentication providers/verifiers and creates an App
     * on this service instance. The direction parameter controls whether the app
     * can send messages, receive messages, both, or neither.
     *
     * # Arguments
     * * `base_name` - The base name for the app (without ID)
     * * `identity_provider_config` - Configuration for proving identity to others
     * * `identity_verifier_config` - Configuration for verifying identity of others
     * * `direction` - Traffic direction: Send, Recv, Bidirectional, or None
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created adapter
     * * `Err(SlimError)` - If adapter creation fails
     */ createAppWithDirectionAsync(name: Name, identityProviderConfig: IdentityProviderConfig, identityVerifierConfig: IdentityVerifierConfig, direction: Direction, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<App>;
    /**
     * Create a new App with SharedSecret authentication (helper function)
     *
     * This is a convenience function for creating a SLIM application using SharedSecret authentication
     * on this service instance.
     *
     * # Arguments
     * * `name` - The base name for the app (without ID)
     * * `secret` - The shared secret string for authentication
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created app
     * * `Err(SlimError)` - If app creation fails
     */ createAppWithSecret(name: Name, secret: string): App;
    /**
     * Create a new App with SharedSecret authentication (async version)
     *
     * This is a convenience function for creating a SLIM application using SharedSecret authentication
     * on this service instance. This is the async version.
     *
     * # Arguments
     * * `name` - The base name for the app (without ID)
     * * `secret` - The shared secret string for authentication
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created app
     * * `Err(SlimError)` - If app creation fails
     */ createAppWithSecretAsync(name: Name, secret: string, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<App>;
    /**
     * Disconnect a client connection by connection ID - blocking version
     */ disconnect(connId: bigint): void;
    /**
     * Get the connection ID for a given endpoint
     */ getConnectionId(endpoint: string): /*u64*/ bigint | undefined;
    /**
     * Get the service identifier/name
     */ getName(): string;
    /**
     * Run the service (starts all configured servers and clients) - blocking version
     */ run(): void;
    /**
     * Run the service (starts all configured servers and clients)
     */ runAsync(asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Start a server with the given configuration - blocking version
     */ runServer(config: ServerConfig): void;
    /**
     * Start a server with the given configuration
     */ runServerAsync(config: ServerConfig, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Shutdown the service gracefully - blocking version
     */ shutdown(): void;
    /**
     * Shutdown the service gracefully
     */ shutdownAsync(asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Stop a server by endpoint - blocking version
     */ stopServer(endpoint: string): void;
};
/**
 * Service wrapper for uniffi bindings
 */
export declare class Service extends UniffiAbstractObject implements ServiceInterface {
    readonly [uniffiTypeNameSymbol] = "Service";
    readonly [destructorGuardSymbol]: UniffiRustArcPtr;
    readonly [pointerLiteralSymbol]: UnsafeMutableRawPointer;
    /**
     * Create a new Service with the given name
     */
    constructor(name: string);
    /**
     * Create a new Service with configuration
     */
    static newWithConfig(name: string, config: ServiceConfig): Service;
    /**
     * Get the service configuration
     */ config(): ServiceConfig;
    /**
     * Connect to a remote endpoint as a client - blocking version
     */ connect(config: ClientConfig): bigint;
    /**
     * Connect to a remote endpoint as a client
     */ connectAsync(config: ClientConfig, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise</*u64*/ bigint>;
    /**
     * Create a new App with authentication configuration (blocking version)
     *
     * This method initializes authentication providers/verifiers and creates a App
     * on this service instance. This is a blocking wrapper around create_app_async.
     *
     * # Arguments
     * * `base_name` - The base name for the app (without ID)
     * * `identity_provider_config` - Configuration for proving identity to others
     * * `identity_verifier_config` - Configuration for verifying identity of others
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created adapter
     * * `Err(SlimError)` - If adapter creation fails
     */ createApp(baseName: Name, identityProviderConfig: IdentityProviderConfig, identityVerifierConfig: IdentityVerifierConfig): App;
    /**
     * Create a new App with authentication configuration (async version)
     *
     * This method initializes authentication providers/verifiers and creates a App
     * on this service instance.
     *
     * # Arguments
     * * `base_name` - The base name for the app (without ID)
     * * `identity_provider_config` - Configuration for proving identity to others
     * * `identity_verifier_config` - Configuration for verifying identity of others
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created adapter
     * * `Err(SlimError)` - If adapter creation fails
     */ createAppAsync(baseName: Name, identityProviderConfig: IdentityProviderConfig, identityVerifierConfig: IdentityVerifierConfig, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<App>;
    /**
     * Create a new App with authentication configuration and traffic direction (blocking version)
     *
     * This method initializes authentication providers/verifiers and creates an App
     * on this service instance. The direction parameter controls whether the app
     * can send messages, receive messages, both, or neither.
     *
     * # Arguments
     * * `base_name` - The base name for the app (without ID)
     * * `identity_provider_config` - Configuration for proving identity to others
     * * `identity_verifier_config` - Configuration for verifying identity of others
     * * `direction` - Traffic direction: Send, Recv, Bidirectional, or None
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created adapter
     * * `Err(SlimError)` - If adapter creation fails
     */ createAppWithDirection(baseName: Name, identityProviderConfig: IdentityProviderConfig, identityVerifierConfig: IdentityVerifierConfig, direction: Direction): App;
    /**
     * Create a new App with authentication configuration and traffic direction (async version)
     *
     * This method initializes authentication providers/verifiers and creates an App
     * on this service instance. The direction parameter controls whether the app
     * can send messages, receive messages, both, or neither.
     *
     * # Arguments
     * * `base_name` - The base name for the app (without ID)
     * * `identity_provider_config` - Configuration for proving identity to others
     * * `identity_verifier_config` - Configuration for verifying identity of others
     * * `direction` - Traffic direction: Send, Recv, Bidirectional, or None
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created adapter
     * * `Err(SlimError)` - If adapter creation fails
     */ createAppWithDirectionAsync(name: Name, identityProviderConfig: IdentityProviderConfig, identityVerifierConfig: IdentityVerifierConfig, direction: Direction, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<App>;
    /**
     * Create a new App with SharedSecret authentication (helper function)
     *
     * This is a convenience function for creating a SLIM application using SharedSecret authentication
     * on this service instance.
     *
     * # Arguments
     * * `name` - The base name for the app (without ID)
     * * `secret` - The shared secret string for authentication
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created app
     * * `Err(SlimError)` - If app creation fails
     */ createAppWithSecret(name: Name, secret: string): App;
    /**
     * Create a new App with SharedSecret authentication (async version)
     *
     * This is a convenience function for creating a SLIM application using SharedSecret authentication
     * on this service instance. This is the async version.
     *
     * # Arguments
     * * `name` - The base name for the app (without ID)
     * * `secret` - The shared secret string for authentication
     *
     * # Returns
     * * `Ok(Arc<App>)` - Successfully created app
     * * `Err(SlimError)` - If app creation fails
     */ createAppWithSecretAsync(name: Name, secret: string, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<App>;
    /**
     * Disconnect a client connection by connection ID - blocking version
     */ disconnect(connId: bigint): void;
    /**
     * Get the connection ID for a given endpoint
     */ getConnectionId(endpoint: string): /*u64*/ bigint | undefined;
    /**
     * Get the service identifier/name
     */ getName(): string;
    /**
     * Run the service (starts all configured servers and clients) - blocking version
     */ run(): void;
    /**
     * Run the service (starts all configured servers and clients)
     */ runAsync(asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Start a server with the given configuration - blocking version
     */ runServer(config: ServerConfig): void;
    /**
     * Start a server with the given configuration
     */ runServerAsync(config: ServerConfig, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Shutdown the service gracefully - blocking version
     */ shutdown(): void;
    /**
     * Shutdown the service gracefully
     */ shutdownAsync(asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Stop a server by endpoint - blocking version
     */ stopServer(endpoint: string): void;
    /**
     * {@inheritDoc uniffi-bindgen-react-native#UniffiAbstractObject.uniffiDestroy}
     */
    uniffiDestroy(): void;
    [Symbol.dispose]: () => void;
    static instanceOf(obj: any): obj is Service;
}
export type SessionInterface = {
    /**
     * Get the session configuration
     */ config(): SessionConfig;
    /**
     * Get the destination name for this session
     */ destination(): Name;
    /**
     * Receive a message from the session (blocking version for FFI)
     *
     * # Arguments
     * * `timeout` - Optional timeout duration
     *
     * # Returns
     * * `Ok(ReceivedMessage)` - Message with context and payload bytes
     * * `Err(SlimError)` - If the receive fails or times out
     */ getMessage(timeout: number | undefined): ReceivedMessage;
    /**
     * Receive a message from the session (async version)
     */ getMessageAsync(timeout: number | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<ReceivedMessage>;
    /**
     * Invite a participant to the session (blocking version)
     *
     * Returns a completion handle that can be awaited to ensure the invitation completes.
     */ invite(participant: Name): CompletionHandle;
    /**
     * Invite a participant and wait for completion (blocking version)
     *
     * This method invites a participant and blocks until the invitation completes.
     */ inviteAndWait(participant: Name): void;
    /**
     * Invite a participant and wait for completion (async version)
     *
     * This method invites a participant and waits until the invitation completes.
     */ inviteAndWaitAsync(participant: Name, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Invite a participant to the session (async version)
     *
     * Returns a completion handle that can be awaited to ensure the invitation completes.
     */ inviteAsync(participant: Name, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<CompletionHandle>;
    /**
     * Check if this session is the initiator
     */ isInitiator(): boolean;
    /**
     * Get the session metadata
     */ metadata(): Map<string, string>;
    /**
     * Get list of participants in the session (blocking version for FFI)
     */ participantsList(): Array<Name>;
    /**
     * Get list of participants in the session
     */ participantsListAsync(asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<Array<Name>>;
    /**
     * Publish a message to the session's destination (blocking version)
     *
     * Returns a completion handle that can be awaited to ensure the message was delivered.
     *
     * # Arguments
     * * `data` - The message payload bytes
     * * `payload_type` - Optional content type identifier
     * * `metadata` - Optional key-value metadata pairs
     *
     * # Returns
     * * `Ok(CompletionHandle)` - Handle to await delivery confirmation
     * * `Err(SlimError)` - If publishing fails
     *
     * # Example
     * ```ignore
     * let completion = session.publish(data, None, None)?;
     * completion.wait()?; // Blocks until message is delivered
     * ```
     */ publish(data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined): CompletionHandle;
    /**
     * Publish a message and wait for completion (blocking version)
     *
     * This method publishes a message and blocks until the delivery completes.
     */ publishAndWait(data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined): void;
    /**
     * Publish a message and wait for completion (async version)
     *
     * This method publishes a message and waits until the delivery completes.
     */ publishAndWaitAsync(data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Publish a message to the session's destination (async version)
     *
     * Returns a completion handle that can be awaited to ensure the message was delivered.
     */ publishAsync(data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<CompletionHandle>;
    /**
     * Publish a reply message to the originator of a received message (blocking version for FFI)
     *
     * This method uses the routing information from a previously received message
     * to send a reply back to the sender. This is the preferred way to implement
     * request/reply patterns.
     *
     * Returns a completion handle that can be awaited to ensure the message was delivered.
     *
     * # Arguments
     * * `message_context` - Context from a message received via `get_message()`
     * * `data` - The reply payload bytes
     * * `payload_type` - Optional content type identifier
     * * `metadata` - Optional key-value metadata pairs
     *
     * # Returns
     * * `Ok(CompletionHandle)` - Handle to await delivery confirmation
     * * `Err(SlimError)` - If publishing fails
     */ publishTo(messageContext: MessageContext, data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined): CompletionHandle;
    /**
     * Publish a reply message and wait for completion (blocking version)
     *
     * This method publishes a reply to a received message and blocks until the delivery completes.
     */ publishToAndWait(messageContext: MessageContext, data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined): void;
    /**
     * Publish a reply message and wait for completion (async version)
     *
     * This method publishes a reply to a received message and waits until the delivery completes.
     */ publishToAndWaitAsync(messageContext: MessageContext, data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Publish a reply message (async version)
     *
     * Returns a completion handle that can be awaited to ensure the message was delivered.
     */ publishToAsync(messageContext: MessageContext, data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<CompletionHandle>;
    /**
     * Low-level publish with full control over all parameters (blocking version for FFI)
     *
     * This is an advanced method that provides complete control over routing and delivery.
     * Most users should use `publish()` or `publish_to()` instead.
     *
     * # Arguments
     * * `destination` - Target name to send to
     * * `fanout` - Number of copies to send (for multicast)
     * * `data` - The message payload bytes
     * * `connection_out` - Optional specific connection ID to use
     * * `payload_type` - Optional content type identifier
     * * `metadata` - Optional key-value metadata pairs
     */ publishWithParams(destination: Name, fanout: number, data: ArrayBuffer, connectionOut: /*u64*/ bigint | undefined, payloadType: string | undefined, metadata: Map<string, string> | undefined): void;
    /**
     * Low-level publish with full control (async version)
     */ publishWithParamsAsync(destination: Name, fanout: number, data: ArrayBuffer, connectionOut: /*u64*/ bigint | undefined, payloadType: string | undefined, metadata: Map<string, string> | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Remove a participant from the session (blocking version)
     *
     * Returns a completion handle that can be awaited to ensure the removal completes.
     */ remove(participant: Name): CompletionHandle;
    /**
     * Remove a participant and wait for completion (blocking version)
     *
     * This method removes a participant and blocks until the removal completes.
     */ removeAndWait(participant: Name): void;
    /**
     * Remove a participant and wait for completion (async version)
     *
     * This method removes a participant and waits until the removal completes.
     */ removeAndWaitAsync(participant: Name, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Remove a participant from the session (async version)
     *
     * Returns a completion handle that can be awaited to ensure the removal completes.
     */ removeAsync(participant: Name, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<CompletionHandle>;
    /**
     * Get the session ID
     */ sessionId(): number;
    /**
     * Get the session type (PointToPoint or Group)
     */ sessionType(): SessionType;
    /**
     * Get the source name for this session
     */ source(): Name;
};
/**
 * Session context for language bindings (UniFFI-compatible)
 *
 * Wraps the session context with proper async access patterns for message reception.
 * Provides both synchronous (blocking) and asynchronous methods for FFI compatibility.
 */
export declare class Session extends UniffiAbstractObject implements SessionInterface {
    readonly [uniffiTypeNameSymbol] = "Session";
    readonly [destructorGuardSymbol]: UniffiRustArcPtr;
    readonly [pointerLiteralSymbol]: UnsafeMutableRawPointer;
    /**
     * Get the session configuration
     */ config(): SessionConfig;
    /**
     * Get the destination name for this session
     */ destination(): Name;
    /**
     * Receive a message from the session (blocking version for FFI)
     *
     * # Arguments
     * * `timeout` - Optional timeout duration
     *
     * # Returns
     * * `Ok(ReceivedMessage)` - Message with context and payload bytes
     * * `Err(SlimError)` - If the receive fails or times out
     */ getMessage(timeout: number | undefined): ReceivedMessage;
    /**
     * Receive a message from the session (async version)
     */ getMessageAsync(timeout: number | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<ReceivedMessage>;
    /**
     * Invite a participant to the session (blocking version)
     *
     * Returns a completion handle that can be awaited to ensure the invitation completes.
     */ invite(participant: Name): CompletionHandle;
    /**
     * Invite a participant and wait for completion (blocking version)
     *
     * This method invites a participant and blocks until the invitation completes.
     */ inviteAndWait(participant: Name): void;
    /**
     * Invite a participant and wait for completion (async version)
     *
     * This method invites a participant and waits until the invitation completes.
     */ inviteAndWaitAsync(participant: Name, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Invite a participant to the session (async version)
     *
     * Returns a completion handle that can be awaited to ensure the invitation completes.
     */ inviteAsync(participant: Name, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<CompletionHandle>;
    /**
     * Check if this session is the initiator
     */ isInitiator(): boolean;
    /**
     * Get the session metadata
     */ metadata(): Map<string, string>;
    /**
     * Get list of participants in the session (blocking version for FFI)
     */ participantsList(): Array<Name>;
    /**
     * Get list of participants in the session
     */ participantsListAsync(asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<Array<Name>>;
    /**
     * Publish a message to the session's destination (blocking version)
     *
     * Returns a completion handle that can be awaited to ensure the message was delivered.
     *
     * # Arguments
     * * `data` - The message payload bytes
     * * `payload_type` - Optional content type identifier
     * * `metadata` - Optional key-value metadata pairs
     *
     * # Returns
     * * `Ok(CompletionHandle)` - Handle to await delivery confirmation
     * * `Err(SlimError)` - If publishing fails
     *
     * # Example
     * ```ignore
     * let completion = session.publish(data, None, None)?;
     * completion.wait()?; // Blocks until message is delivered
     * ```
     */ publish(data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined): CompletionHandle;
    /**
     * Publish a message and wait for completion (blocking version)
     *
     * This method publishes a message and blocks until the delivery completes.
     */ publishAndWait(data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined): void;
    /**
     * Publish a message and wait for completion (async version)
     *
     * This method publishes a message and waits until the delivery completes.
     */ publishAndWaitAsync(data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Publish a message to the session's destination (async version)
     *
     * Returns a completion handle that can be awaited to ensure the message was delivered.
     */ publishAsync(data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<CompletionHandle>;
    /**
     * Publish a reply message to the originator of a received message (blocking version for FFI)
     *
     * This method uses the routing information from a previously received message
     * to send a reply back to the sender. This is the preferred way to implement
     * request/reply patterns.
     *
     * Returns a completion handle that can be awaited to ensure the message was delivered.
     *
     * # Arguments
     * * `message_context` - Context from a message received via `get_message()`
     * * `data` - The reply payload bytes
     * * `payload_type` - Optional content type identifier
     * * `metadata` - Optional key-value metadata pairs
     *
     * # Returns
     * * `Ok(CompletionHandle)` - Handle to await delivery confirmation
     * * `Err(SlimError)` - If publishing fails
     */ publishTo(messageContext: MessageContext, data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined): CompletionHandle;
    /**
     * Publish a reply message and wait for completion (blocking version)
     *
     * This method publishes a reply to a received message and blocks until the delivery completes.
     */ publishToAndWait(messageContext: MessageContext, data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined): void;
    /**
     * Publish a reply message and wait for completion (async version)
     *
     * This method publishes a reply to a received message and waits until the delivery completes.
     */ publishToAndWaitAsync(messageContext: MessageContext, data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Publish a reply message (async version)
     *
     * Returns a completion handle that can be awaited to ensure the message was delivered.
     */ publishToAsync(messageContext: MessageContext, data: ArrayBuffer, payloadType: string | undefined, metadata: Map<string, string> | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<CompletionHandle>;
    /**
     * Low-level publish with full control over all parameters (blocking version for FFI)
     *
     * This is an advanced method that provides complete control over routing and delivery.
     * Most users should use `publish()` or `publish_to()` instead.
     *
     * # Arguments
     * * `destination` - Target name to send to
     * * `fanout` - Number of copies to send (for multicast)
     * * `data` - The message payload bytes
     * * `connection_out` - Optional specific connection ID to use
     * * `payload_type` - Optional content type identifier
     * * `metadata` - Optional key-value metadata pairs
     */ publishWithParams(destination: Name, fanout: number, data: ArrayBuffer, connectionOut: /*u64*/ bigint | undefined, payloadType: string | undefined, metadata: Map<string, string> | undefined): void;
    /**
     * Low-level publish with full control (async version)
     */ publishWithParamsAsync(destination: Name, fanout: number, data: ArrayBuffer, connectionOut: /*u64*/ bigint | undefined, payloadType: string | undefined, metadata: Map<string, string> | undefined, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Remove a participant from the session (blocking version)
     *
     * Returns a completion handle that can be awaited to ensure the removal completes.
     */ remove(participant: Name): CompletionHandle;
    /**
     * Remove a participant and wait for completion (blocking version)
     *
     * This method removes a participant and blocks until the removal completes.
     */ removeAndWait(participant: Name): void;
    /**
     * Remove a participant and wait for completion (async version)
     *
     * This method removes a participant and waits until the removal completes.
     */ removeAndWaitAsync(participant: Name, asyncOpts_?: {
        signal: AbortSignal;
    }): void;
    /**
     * Remove a participant from the session (async version)
     *
     * Returns a completion handle that can be awaited to ensure the removal completes.
     */ removeAsync(participant: Name, asyncOpts_?: {
        signal: AbortSignal;
    }): Promise<CompletionHandle>;
    /**
     * Get the session ID
     */ sessionId(): number;
    /**
     * Get the session type (PointToPoint or Group)
     */ sessionType(): SessionType;
    /**
     * Get the source name for this session
     */ source(): Name;
    /**
     * {@inheritDoc uniffi-bindgen-react-native#UniffiAbstractObject.uniffiDestroy}
     */
    uniffiDestroy(): void;
    [Symbol.dispose]: () => void;
    static instanceOf(obj: any): obj is Session;
}
/**
 * Create a new Service with builder pattern
 */
export declare function createService(name: string): Service;
/**
 * Create a new Service with configuration
 */
export declare function createServiceWithConfig(name: string, config: ServiceConfig): Service;
/**
 * Get detailed build information
 */
export declare function getBuildInfo(): BuildInfo;
/**
 * Get the global service instance (creates it if it doesn't exist)
 *
 * This returns a reference to the shared global service that can be used
 * across the application. All calls to this function return the same service instance.
 */
export declare function getGlobalService(): Service;
/**
 * Returns references to all global services.
 * If not initialized, initializes with defaults first.
 */
export declare function getServices(): Array<Service>;
/**
 * Get the version of the SLIM bindings (simple string)
 */
export declare function getVersion(): string;
/**
 * Initialize SLIM bindings from a configuration file
 *
 * This function:
 * 1. Loads the configuration file
 * 2. Initializes the crypto provider
 * 3. Sets up tracing/logging exactly as the main SLIM application does
 * 4. Initializes the global runtime with configuration from the file
 * 5. Initializes and starts the global service with servers/clients from config
 *
 * This must be called before using any SLIM bindings functionality.
 * It's safe to call multiple times - subsequent calls will be ignored.
 *
 * # Arguments
 * * `config_path` - Path to the YAML configuration file
 *
 * # Returns
 * * `Ok(())` - Successfully initialized
 * * `Err(SlimError)` - If initialization fails
 *
 * # Example
 * ```ignore
 * initialize_from_config("/path/to/config.yaml")?;
 * ```
 */
export declare function initializeFromConfig(configPath: string): void;
/**
 * Initialize SLIM bindings with custom configuration structs
 *
 * This function allows you to programmatically configure SLIM bindings by passing
 * configuration structs directly, without needing a config file.
 *
 * # Arguments
 * * `runtime_config` - Runtime configuration (thread count, naming, etc.)
 * * `tracing_config` - Tracing/logging configuration
 * * `service_config` - Service configuration (node ID, group name, etc.)
 *
 * # Returns
 * * `Ok(())` - Successfully initialized
 * * `Err(SlimError)` - If initialization fails
 *
 * # Example
 * ```ignore
 * let runtime_config = new_runtime_config();
 * let tracing_config = new_tracing_config();
 * let mut service_config = new_service_config();
 * service_config.node_id = Some("my-node".to_string());
 *
 * initialize_with_configs(runtime_config, tracing_config, service_config)?;
 * ```
 */
export declare function initializeWithConfigs(runtimeConfig: RuntimeConfig, tracingConfig: TracingConfig, serviceConfig: Array<ServiceConfig>): void;
/**
 * Initialize SLIM bindings with default configuration
 *
 * This is a convenience function that initializes the bindings with:
 * - Default runtime configuration
 * - Default tracing/logging configuration
 * - Initialized crypto provider
 * - Default global service (no servers/clients)
 *
 * Use `initialize_from_config` for file-based configuration or
 * `initialize_with_configs` for programmatic configuration.
 */
export declare function initializeWithDefaults(): void;
/**
 * Check if SLIM bindings have been initialized
 */
export declare function isInitialized(): boolean;
/**
 * Create a new DataplaneConfig
 */
export declare function newDataplaneConfig(): DataplaneConfig;
/**
 * Create a new insecure client config (no TLS)
 */
export declare function newInsecureClientConfig(endpoint: string): ClientConfig;
/**
 * Create a new insecure server config (no TLS)
 */
export declare function newInsecureServerConfig(endpoint: string): ServerConfig;
/**
 * Create a new BindingsRuntimeConfig with default values
 */
export declare function newRuntimeConfig(): RuntimeConfig;
/**
 * Create a new BindingsRuntimeConfig with custom values
 */
export declare function newRuntimeConfigWith(nCores: bigint, threadName: string, drainTimeout: number): RuntimeConfig;
/**
 * Create a new server config with the given endpoint and default values
 */
export declare function newServerConfig(endpoint: string): ServerConfig;
/**
 * Create a new BindingsServiceConfig with default values
 */
export declare function newServiceConfig(): ServiceConfig;
/**
 * Create a new BindingsServiceConfig with custom values
 */
export declare function newServiceConfigWith(nodeId: string | undefined, groupName: string | undefined, dataplane: DataplaneConfig): ServiceConfig;
/**
 * Create a new ServiceConfiguration
 */
export declare function newServiceConfiguration(): ServiceConfig;
/**
 * Create a new BindingsTracingConfig with default values
 */
export declare function newTracingConfig(): TracingConfig;
/**
 * Create a new BindingsTracingConfig with custom values
 */
export declare function newTracingConfigWith(logLevel: string, displayThreadNames: boolean, displayThreadIds: boolean, filters: Array<string>): TracingConfig;
/**
 * Perform graceful shutdown operations (blocking version)
 *
 * This is a blocking wrapper around the async `shutdown()` function for use from
 * synchronous contexts or language bindings that don't support async.
 *
 * # Returns
 * * `Ok(())` - Successfully shut down
 * * `Err(SlimError)` - If shutdown fails
 */
export declare function shutdownBlocking(): void;
