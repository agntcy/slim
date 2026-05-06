package db

import "encoding/json"

// Struct for the client configuration.
// This struct contains the endpoint, origin, compression type, rate limit,
// TLS settings, keepalive settings, timeout settings, buffer size settings,
// headers, and auth settings.
// The client configuration can be converted to a tonic channel.
type ClientConnectionConfig struct {
	// Auth configuration for outgoing RPCs.
	Auth *Auth `json:"auth,omitempty"`
	// Backoff retry configuration.
	Backoff *BackoffConfig `json:"backoff,omitempty"`
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

// Backoff retry configuration.
// Matches dataplane tagged enum JSON with flattened fields.
type BackoffConfig struct {
	// One of: fixed_interval, exponential
	Type string `json:"type"`
	// Flattened fixed-interval fields when Type == "fixed_interval"
	*FixedIntervalBackoffConfig
	// Flattened exponential fields when Type == "exponential"
	*ExponentialBackoffConfig
}

// Fixed-interval backoff retry configuration.
type FixedIntervalBackoffConfig struct {
	Interval    string `json:"interval"`
	MaxAttempts *int   `json:"max_attempts,omitempty"`
}

// Exponential backoff retry configuration.
type ExponentialBackoffConfig struct {
	Base        uint64 `json:"base"`
	Factor      uint64 `json:"factor"`
	MaxDelay    string `json:"max_delay"`
	MaxAttempts *int   `json:"max_attempts,omitempty"`
	Jitter      *bool  `json:"jitter,omitempty"`
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
	// Duration used by the AddJwtLayer cache for re-fetching the token.
	// This duration bounds the "validity" window before re-reading from file.
	Duration *string `json:"duration,omitempty"`
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

// SPIRE authentication configuration.
type SpireAuth struct {
	// Path to the SPIFFE Workload API socket (None => use SPIFFE_ENDPOINT_SOCKET env var)
	SocketPath *string `json:"socket_path,omitempty"`
	// Optional target SPIFFE ID when requesting JWT SVIDs
	TargetSpiffeID *string `json:"target_spiffe_id,omitempty"`
	// Audiences to request / verify for JWT SVIDs
	JwtAudiences *[]string `json:"jwt_audiences,omitempty"`
	// Optional trust domains override for X.509 bundle retrieval.
	TrustDomains *[]string `json:"trust_domains,omitempty"`
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
	AuthTypeBasic     AuthEnum = "basic"
	AuthTypeStaticJwt AuthEnum = "static_jwt"
	AuthTypeJWT       AuthEnum = "jwt"
	AuthTypeSpire     AuthEnum = "spire"
	None              AuthEnum = "none"
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
	// Discriminator for the tagged auth union.
	Type AuthEnum `json:"type"`
	// Variant payloads; marshaled/unmarshaled as flattened fields.
	Basic     *Basic     `json:"-"`
	StaticJwt *StaticJwt `json:"-"`
	Jwt       *Jwt       `json:"-"`
	Spire     *SpireAuth `json:"-"`
}

// MarshalJSON serializes Auth in the same tagged + flattened shape used by
// Rust's AuthenticationConfig (`#[serde(tag = "type")]`).
func (a Auth) MarshalJSON() ([]byte, error) {
	typ := a.Type
	if typ == "" {
		typ = None
	}

	payload := map[string]interface{}{
		"type": typ,
	}

	switch typ {
	case AuthTypeBasic:
		if a.Basic != nil {
			payload["username"] = a.Basic.Username
			payload["password"] = a.Basic.Password
		}
	case AuthTypeStaticJwt:
		if a.StaticJwt != nil {
			payload["file"] = a.StaticJwt.File
			if a.StaticJwt.Duration != nil {
				payload["duration"] = *a.StaticJwt.Duration
			}
		}
	case AuthTypeJWT:
		if a.Jwt != nil {
			if a.Jwt.Claims != nil {
				payload["claims"] = a.Jwt.Claims
			}
			if a.Jwt.Duration != nil {
				payload["duration"] = *a.Jwt.Duration
			}
			if a.Jwt.Key != nil {
				payload["key"] = a.Jwt.Key
			}
		}
	case AuthTypeSpire:
		if a.Spire != nil {
			if a.Spire.SocketPath != nil {
				payload["socket_path"] = *a.Spire.SocketPath
			}
			if a.Spire.TargetSpiffeID != nil {
				payload["target_spiffe_id"] = *a.Spire.TargetSpiffeID
			}
			if a.Spire.JwtAudiences != nil {
				payload["jwt_audiences"] = *a.Spire.JwtAudiences
			}
			if a.Spire.TrustDomains != nil {
				payload["trust_domains"] = *a.Spire.TrustDomains
			}
		}
	case None:
		// no extra fields
	}

	return json.Marshal(payload)
}

// UnmarshalJSON deserializes tagged + flattened auth objects.
func (a *Auth) UnmarshalJSON(data []byte) error {
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}

	if t, ok := raw["type"]; ok {
		if err := json.Unmarshal(t, &a.Type); err != nil {
			return err
		}
	} else {
		a.Type = None
	}

	switch a.Type {
	case AuthTypeBasic:
		var basic Basic
		if v, ok := raw["username"]; ok {
			if err := json.Unmarshal(v, &basic.Username); err != nil {
				return err
			}
		}
		if v, ok := raw["password"]; ok {
			if err := json.Unmarshal(v, &basic.Password); err != nil {
				return err
			}
		}
		a.Basic = &basic
	case AuthTypeStaticJwt:
		var sj StaticJwt
		if v, ok := raw["file"]; ok {
			if err := json.Unmarshal(v, &sj.File); err != nil {
				return err
			}
		}
		if v, ok := raw["duration"]; ok {
			var d string
			if err := json.Unmarshal(v, &d); err != nil {
				return err
			}
			sj.Duration = &d
		}
		a.StaticJwt = &sj
	case AuthTypeJWT:
		var jwt Jwt
		if v, ok := raw["claims"]; ok {
			var c Claims
			if err := json.Unmarshal(v, &c); err != nil {
				return err
			}
			jwt.Claims = &c
		}
		if v, ok := raw["duration"]; ok {
			var d string
			if err := json.Unmarshal(v, &d); err != nil {
				return err
			}
			jwt.Duration = &d
		}
		if v, ok := raw["key"]; ok {
			var k JwtKey
			if err := json.Unmarshal(v, &k); err != nil {
				return err
			}
			jwt.Key = &k
		}
		a.Jwt = &jwt
	case AuthTypeSpire:
		var spire SpireAuth
		if v, ok := raw["socket_path"]; ok {
			var s string
			if err := json.Unmarshal(v, &s); err != nil {
				return err
			}
			spire.SocketPath = &s
		}
		if v, ok := raw["target_spiffe_id"]; ok {
			var s string
			if err := json.Unmarshal(v, &s); err != nil {
				return err
			}
			spire.TargetSpiffeID = &s
		}
		if v, ok := raw["jwt_audiences"]; ok {
			var audiences []string
			if err := json.Unmarshal(v, &audiences); err != nil {
				return err
			}
			spire.JwtAudiences = &audiences
		}
		if v, ok := raw["trust_domains"]; ok {
			var domains []string
			if err := json.Unmarshal(v, &domains); err != nil {
				return err
			}
			spire.TrustDomains = &domains
		}
		a.Spire = &spire
	case None:
		// no-op
	}

	return nil
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

type SeverTLSConfig struct {
	// CA source configuration
	CaSource *CaSource `json:"ca_source,omitempty"`
	// If true, load system CA certificates pool in addition to the certificates
	// configured in this struct.
	IncludeSystemCACertsPool *bool `json:"include_system_ca_certs_pool,omitempty"`
	// TLS source configuration
	Source *TLSSource `json:"source,omitempty"`
	// The TLS version to use. If not set, the default is "tls1.3".
	// The value must be either "tls1.2" or "tls1.3".
	// (optional)
	TLSVersion *string `json:"tls_version,omitempty"`
}
