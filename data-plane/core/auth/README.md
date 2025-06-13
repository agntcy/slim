# AGNTCY Slim Auth

This crate provides authentication and authorization capabilities for the AGNTCY
Slim platform, with a focus on JWT (JSON Web Token) authentication.

## Features

- JWT token creation and verification
- Builder pattern for fluent JWT configuration
- Flexible key resolution for JWT verification
- Support for OpenID Connect Discovery
- JWKS (JSON Web Key Set) integration
- Asynchronous verification for improved performance

## JWT Authentication

The JWT implementation provides a convenient way to create and verify JWTs with
support for automatic key resolution from OpenID Connect providers. The
implementation uses a fluent builder pattern to configure and construct JWT
instances.

### Basic Usage

```rust
use agntcy_slim_auth::jwt::{Jwt, StandardClaims};
use agntcy_slim_auth::traits::{Signer, Verifier};
use jsonwebtoken_aws_lc::Algorithm;
use std::collections::HashMap;
use std::time::Duration;

// Create a signer with the private key
let signer = Jwt::builder()
    .issuer("https://mycompany.com")
    .audience("api-users")
    .subject("auth")
    .private_key(Algorithm::HS256, "your-secret-key")
    .build()
    .unwrap();

// Create claims
let mut custom_claims = HashMap::new();
custom_claims.insert("role".to_string(), serde_json::Value::String("admin".to_string()));
let claims = signer.create_standard_claims(Some(custom_claims));

// Sign the claims
let token = signer.sign(&claims).unwrap();

// Create a separate verifier with the public key
let mut verifier = Jwt::builder()
    .issuer("https://mycompany.com")
    .audience("api-users")
    .subject("auth")
    .public_key(Algorithm::HS256, "your-secret-key")
    .build()
    .unwrap();

// Verify the token
let verified_claims: StandardClaims = verifier.verify(&token).await.unwrap();
println!("Token verified: {}", verified_claims.iss.unwrap());
```

### Creating a Signer and Verifier

The builder provides separate methods for creating signers and verifiers:

```rust
use agntcy_slim_auth::jwt::Jwt;
use agntcy_slim_auth::traits::{Signer, Verifier};
use jsonwebtoken_aws_lc::Algorithm;

// Create a signer using the private key
let signer = Jwt::builder()
    .issuer("https://mycompany.com")
    .audience("api-users")
    .subject("auth")
    .private_key(Algorithm::HS256, "your-secret-key")
    .build()
    .unwrap();

// Create a verifier using the public key
let verifier = Jwt::builder()
    .issuer("https://mycompany.com")
    .audience("api-users")
    .subject("auth")
    .public_key(Algorithm::HS256, "your-secret-key")
    .build()
    .unwrap();
```

### Auto Key Resolution

The library supports automatic key resolution from OpenID Connect providers,
allowing you to verify tokens from external identity providers like Auth0, Azure
AD, or Okta.

```rust
use agntcy_slim_auth::jwt::{Jwt, StandardClaims};
use agntcy_slim_auth::traits::{Signer, Verifier};
use std::time::Duration;

// Create a JWT instance with auto key resolution
let mut verifier = Jwt::builder()
    .issuer("https://your-tenant.auth0.com/")
    .audience("your-api")
    .subject("auth")
    .auto_resolve_keys(true)
    .build()
    .unwrap();

// Verify a token asynchronously
let token = "..."; // Token received from the client
let claims: StandardClaims = verifier.verify(&token).await.unwrap();
```

### OpenID Connect Discovery

The key resolver will first attempt to discover the JWKS URI using the standard
OpenID Connect discovery endpoint (`/.well-known/openid-configuration`) and will
fall back to the standard JWKS location (`/.well-known/jwks.json`) if needed.

### Simplified Builder Pattern

The builder uses a more straightforward approach to configuration:

1. Set properties like issuer, audience, and subject
2. Configure key information or auto-resolution
3. Call build() to create the JWT instance

This provides a clean and intuitive API while still ensuring appropriate
validation during construction.

```rust
// Simple, readable configuration
let jwt = Jwt::builder()
    .issuer("https://mycompany.com")
    .audience("api-users")
    .subject("auth")
    .private_key(Algorithm::HS256, "your-secret-key")
    .build()
    .unwrap();
```

The builder pattern returns specific types based on the configuration provided:

- When using `private_key()`, it returns a `Signer` trait object
- When using `public_key()` or `auto_resolve_keys()`, it returns a `Verifier`
  trait object

## License

Apache-2.0
