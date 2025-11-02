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
	// Origin for the client.
	Origin *string `json:"origin,omitempty"`
	// Rate Limits
	RateLimit *string `json:"rate_limit,omitempty"`
	// Timeout per request.
	RequestTimeout *string `json:"request_timeout,omitempty"`
	// TLS client configuration.
	TLS *TLS `json:"tls,omitempty"`
}

// Basic authentication configuration.
//
// Bearer authentication configuration.
//
// JWT authentication configuration.
type AuthClass struct {
	Basic  *Basic  `json:"basic,omitempty"`
	Bearer *Bearer `json:"bearer,omitempty"`
	Jwt    *Jwt    `json:"jwt,omitempty"`
}

type Basic struct {
	// Origin for the client.
	Password string `json:"password"`
	// The target the client will connect to.
	Username string `json:"username"`
}

type Bearer struct {
	Token string `json:"token"`
}

type Jwt struct {
	// Claims
	Claims *Claims `json:"claims,omitempty"`
	// JWT Duration (will become exp: now() + duration)
	Duration *Duration `json:"duration,omitempty"`
}

// Claims
type Claims struct {
	// JWT audience
	Audience *string `json:"audience"`
	// JWT Issuer
	Issuer *string `json:"issuer"`
	// JWT Subject
	Subject *string `json:"subject"`
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
	HTTPClient2Keepalive *string `json:"http_client_2_keepalive,omitempty"`
	// Whether to permit keepalive without an active stream
	KeepAliveWhileIdle *bool `json:"keep_alive_while_idle,omitempty"`
	// The duration of the keepalive time for TCP
	TCPKeepalive *string `json:"tcp_keepalive,omitempty"`
	// The timeout duration for the keepalive
	Timeout *string `json:"timeout,omitempty"`
}

// TLS client configuration.
type TLS struct {
	// Path to the CA cert. For a client this verifies the server certificate.
	// For a server this verifies client certificates. If empty uses system root CA.
	// (optional)
	CAFile *string `json:"ca_file,omitempty"`
	// In memory PEM encoded cert. (optional)
	CAPem *string `json:"ca_pem,omitempty"`
	// Path to the TLS cert to use for TLS required connections. (optional)
	CERTFile *string `json:"cert_file,omitempty"`
	// In memory PEM encoded TLS cert to use for TLS required connections. (optional)
	CERTPem *string `json:"cert_pem,omitempty"`
	// If true, load system CA certificates pool in addition to the certificates
	// configured in this struct.
	IncludeSystemCACertsPool *bool `json:"include_system_ca_certs_pool,omitempty"`
	// In gRPC and HTTP when set to true, this is used to disable the client transport security.
	// (optional, default false)
	Insecure *bool `json:"insecure,omitempty"`
	// InsecureSkipVerify will enable TLS but not verify the server certificate.
	InsecureSkipVerify *bool `json:"insecure_skip_verify,omitempty"`
	// Path to the TLS key to use for TLS required connections. (optional)
	KeyFile *string `json:"key_file,omitempty"`
	// In memory PEM encoded TLS key to use for TLS required connections. (optional)
	KeyPem *string `json:"key_pem,omitempty"`
	// ReloadInterval specifies the duration after which the certificate will be reloaded
	// If not set, it will never be reloaded
	ReloadInterval *Duration `json:"reload_interval,omitempty"`
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
