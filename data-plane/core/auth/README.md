# AGNTCY Slim Auth

This crate provides authentication and authorization capabilities for the AGNTCY Slim platform, with a focus on JWT (JSON Web Token) authentication.

## Features

- JWT token creation and verification
- Builder pattern for fluent JWT configuration
- Flexible key resolution for JWT verification
- Support for OpenID Connect Discovery
- JWKS (JSON Web Key Set) integration
- Asynchronous verification for improved performance

## JWT Authentication

The JWT implementation provides a convenient way to create and verify JWTs with support for automatic key resolution from OpenID Connect providers.

### Basic Usage

```rust
use agntcy_slim_auth::jwt::{Jwt, StandardClaims};
use agntcy_slim_auth::traits::{Signer, Verifier};
use jsonwebtoken::Algorithm;
use std::collections::HashMap;

// Create a JWT instance using the builder pattern
let jwt = Jwt::builder()
    .issuer("https://mycompany.com")
    .audience("api-users")
    .subject("auth")
    .private_key("your-secret-key")
    .algorithm(Algorithm::HS256)
    .build()?;

// Create claims
let mut custom_claims = HashMap::new();
custom_claims.insert("role".to_string(), serde_json::Value::String("admin".to_string()));
let claims = jwt.create_standard_claims(Some(custom_claims));

// Sign the claims
let token = jwt.sign(&claims)?;

// Verify the token
let verified_claims: StandardClaims = jwt.verify(&token)?;
println!("Token verified: {}", verified_claims.sub);
```

### Auto Key Resolution

The library supports automatic key resolution from OpenID Connect providers, allowing you to verify tokens from external identity providers like Auth0, Azure AD, or Okta.

```rust
use agntcy_slim_auth::jwt::Jwt;
use agntcy_slim_auth::traits::AsyncVerifier;

// Create a JWT instance with auto key resolution
let jwt = Jwt::builder()
    .issuer("https://your-tenant.auth0.com/")
    .audience("your-api")
    .auto_resolve_keys(true) // Enable automatic key resolution
    .build()?;

// Verify a token asynchronously
let token = "..."; // Token received from the client
let claims = jwt.verify_async::<StandardClaims>(token).await?;
```

### OpenID Connect Discovery

The key resolver will first attempt to discover the JWKS URI using the standard OpenID Connect discovery endpoint (`/.well-known/openid-configuration`) and will fall back to the standard JWKS location (`/.well-known/jwks.json`) if needed.

### Custom Key Resolver

You can create a custom key resolver with specific settings:

```rust
use agntcy_slim_auth::jwt::Jwt;
use agntcy_slim_auth::jwks::KeyResolver;
use std::time::Duration;

// Create a custom key resolver
let resolver = KeyResolver::new()
    .with_jwks_ttl(Duration::from_secs(3600)); // 1 hour cache TTL

// Use it in a JWT instance
let jwt = Jwt::builder()
    .issuer("https://identity.mycompany.com")
    .audience("api")
    .key_resolver(resolver)
    .build()?;
```

## License

Apache-2.0