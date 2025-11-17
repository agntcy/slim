package routes

// Struct for the client configuration.
// This struct contains the endpoint, origin, compression type, rate limit,
// TLS settings, keepalive settings, timeout settings, buffer size settings,
// headers, and auth settings.
// The client configuration can be converted to a tonic channel.
type ConnectionConfig struct {
	// Auth configuration for outgoing RPCs.
	Auth *Auth `json:"auth,omitempty"`
	// ReadBufferSize.
	BufferSize *int64 `json:"buffer_size,omitempty"`
	// Compression type - TODO(msardara): not implemented yet.
	Compression *CompressionEnum `json:"compression,omitempty"`
	// Timeout for the connection.
	ConnectTimeout *string `json:"connect_timeout,omitempty"`
	// The target the client will connect to.
	Endpoint string `json:"endpoint"`
	// The headers associated with gRPC requests.
	Headers map[string]string `json:"headers,omitempty"`
	// Keepalive parameters.
	Keepalive *KeepaliveClass `json:"keepalive,omitempty"`
	// Arbitrary user-provided metadata.
	Metadata *MetadataMap `json:"metadata,omitempty"`
	// Origin for the client.
	Origin *string `json:"origin,omitempty"`
	// HTTP Proxy configuration.
	Proxy *ProxyConfig `json:"proxy,omitempty"`
	// Rate Limits
	RateLimit *string `json:"rate_limit,omitempty"`
	// Timeout per request.
	RequestTimeout *string `json:"request_timeout,omitempty"`
	// Optional TLS SNI server name override. If set, this value is used for TLS
	// server name verification (SNI) instead of the host extracted from endpoint/origin.
	ServerName *string `json:"server_name,omitempty"`
	// TLS client configuration.
	TLS *TLS `json:"tls,omitempty"`
}

// Basic authentication configuration.
type Basic struct {
	// The username the client will use to authenticate.
	Username string `json:"username"`
	// The password for the username.
	Password string `json:"password"`
}

// Bearer authentication configuration (static JWT from file).
type StaticJwt struct {
	// Path to a file containing the token (auto-reloaded on change)
	File string `json:"file"`
	// Duration (in seconds) used by the AddJwtLayer cache for re-fetching the token.
	// This duration bounds the "validity" window before re-reading from file.
	Duration *int64 `json:"duration,omitempty"`
}

// JWT authentication configuration.
type Jwt struct {
	// Claims
	Claims *Claims `json:"claims,omitempty"`
	// JWT Duration (will become exp: now() + duration)
	Duration *string `json:"duration,omitempty"`
	// One of: `encoding`, `decoding`, or `autoresolve`
	// Encoding key is used for signing JWTs (client-side).
	// Decoding key is used for verifying JWTs (server-side).
	// Autoresolve is used to automatically resolve the key based on the claims.
	Key *JwtKey `json:"key"`
}

// JWT authentication configuration.
type AuthClass struct {
	Basic     *Basic     `json:"basic,omitempty"`
	StaticJwt *StaticJwt `json:"static_jwt,omitempty"`
	Jwt       *Jwt       `json:"jwt,omitempty"`
}

// Claims
type Claims struct {
	// JWT audience
	Audience *[]string `json:"audience,omitempty"`
	// Custom claims
	CustomClaims *MetadataMap `json:"custom_claims,omitempty"`
	// JWT Issuer
	Issuer *string `json:"issuer,omitempty"`
	// JWT Subject
	Subject *string `json:"subject,omitempty"`
}

// JWT Duration (will become exp: now() + duration)
type Duration struct {
	Nanos int64 `json:"nanos"`
	Secs  int64 `json:"secs"`
}

// Keepalive configuration for the client.
// This struct contains the keepalive time for TCP and HTTP2,
// the timeout duration for the keepalive, and whether to permit
// keepalive without an active stream.
type KeepaliveClass struct {
	// The duration of the keepalive time for HTTP2
	HTTP2Keepalive *string `json:"http2_keepalive,omitempty"`
	// Whether to permit keepalive without an active stream
	KeepAliveWhileIdle *bool `json:"keep_alive_while_idle,omitempty"`
	// The duration of the keepalive time for TCP
	TCPKeepalive *string `json:"tcp_keepalive,omitempty"`
	// The timeout duration for the keepalive
	Timeout *string `json:"timeout,omitempty"`
}

// TLS client configuration.
type TLS struct {
	// CA source configuration
	CaSource *CaSource `json:"ca_source,omitempty"`
	// If true, load system CA certificates pool in addition to the certificates
	// configured in this struct.
	IncludeSystemCACertsPool *bool `json:"include_system_ca_certs_pool,omitempty"`
	// In gRPC and HTTP when set to true, this is used to disable the client transport security.
	// (optional, default false)
	Insecure *bool `json:"insecure,omitempty"`
	// InsecureSkipVerify will enable TLS but not verify the server certificate.
	InsecureSkipVerify *bool `json:"insecure_skip_verify,omitempty"`
	// ReloadInterval specifies the duration after which the certificate will be reloaded
	// If not set, it will never be reloaded
	ReloadInterval *Duration `json:"reload_interval,omitempty"`
	// TLS source configuration
	Source *TLSSource `json:"source,omitempty"`
	// The TLS version to use. If not set, the default is "tls1.3".
	// The value must be either "tls1.2" or "tls1.3".
	// (optional)
	TLSVersion *string `json:"tls_version,omitempty"`
}

// None
type AuthEnum string

const (
	None AuthEnum = "none"
)

// CompressionType represents the supported compression types for gRPC messages.
// The supported types are: Gzip, Zlib, Deflate, Snappy, Zstd, Lz4, None, and Empty.
// The default type is None.
type CompressionEnum string

const (
	CoordinatNone CompressionEnum = "None"
	Deflate       CompressionEnum = "Deflate"
	Empty         CompressionEnum = "Empty"
	Gzip          CompressionEnum = "Gzip"
	Lz4           CompressionEnum = "Lz4"
	Snappy        CompressionEnum = "Snappy"
	Zlib          CompressionEnum = "Zlib"
	Zstd          CompressionEnum = "Zstd"
)

// Auth configuration for outgoing RPCs.
//
// Enum holding one configuration for the client.
type Auth struct {
	AuthClass *AuthClass
	Enum      *AuthEnum
}

// A generic metadata map. Newtype with a flattened map so that serde encodes
// just a JSON/YAML object and not an inner field name.
type MetadataMap map[string]MetadataValue

// A generic metadata value.
//
// Supported variants:
// - String
// - Number (serde_json::Number – can represent integer & floating point)
// - List (Vec<MetadataValue>)
// - Map (nested MetadataMap)
type MetadataValue interface{}

// HTTP Proxy configuration.
type ProxyConfig struct {
	// Headers to send with proxy requests
	Headers map[string]string `json:"headers,omitempty"`
	// Optional password for proxy authentication
	Password *string `json:"password,omitempty"`
	// TLS client configuration.
	TLS *TLS `json:"tls,omitempty"`
	// The HTTP proxy URL (e.g., "http://proxy.example.com:8080")
	// If empty, the system proxy settings will be used.
	URL *string `json:"url,omitempty"`
	// Optional username for proxy authentication
	Username *string `json:"username,omitempty"`
}

// CA source configuration
type CaSource struct {
	Type string `json:"type"`
	// For type "file"
	Path *string `json:"path,omitempty"`
	// For type "pem"
	Data *string `json:"data,omitempty"`
	// For type "spire"
	JwtAudiences   *[]string `json:"jwt_audiences,omitempty"`
	SocketPath     *string   `json:"socket_path,omitempty"`
	TargetSpiffeID *string   `json:"target_spiffe_id,omitempty"`
	TrustDomains   *[]string `json:"trust_domains,omitempty"`
}

// TLS source configuration
type TLSSource struct {
	Type string `json:"type"`
	// For type "pem" or "file"
	Cert *string `json:"cert,omitempty"`
	Key  *string `json:"key,omitempty"`
	// For type "spire"
	JwtAudiences   *[]string `json:"jwt_audiences,omitempty"`
	SocketPath     *string   `json:"socket_path,omitempty"`
	TargetSpiffeID *string   `json:"target_spiffe_id,omitempty"`
	TrustDomains   *[]string `json:"trust_domains,omitempty"`
}

// JWT key configuration
type JwtKey struct {
	Type string `json:"type"`
	// For type "encoding" or "decoding"
	Algorithm *string  `json:"algorithm,omitempty"`
	Format    *string  `json:"format,omitempty"`
	Key       *KeyData `json:"key,omitempty"`
}

// Key data configuration
type KeyData struct {
	// For type "data" - String with encoded key(s)
	Data *string `json:"data,omitempty"`
	// For type "file" - File path to the key(s)
	File *string `json:"file,omitempty"`
}
