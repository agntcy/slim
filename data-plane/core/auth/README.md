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
implementation uses a state machine pattern to enforce the correct sequence of
method calls at compile time.

### Basic Usage

```rust
use agntcy_slim_auth::jwt::{Jwt, StandardClaims};
use agntcy_slim_auth::traits::{Signer, Verifier};
use jsonwebtoken_aws_lc::Algorithm;
use std::collections::HashMap;
use std::time::Duration;

// Create a JWT instance using the builder pattern with type state
let jwt = Jwt::builder()
    .issuer("https://mycompany.com")
    .audience("api-users")
    .subject("auth")
    .with_required_info().unwrap()    // Transition to RequiredInfo state
    .private_key(Algorithm::HS256, "your-secret-key")
    .build()
    .unwrap();

// Create claims
let mut custom_claims = HashMap::new();
custom_claims.insert("role".to_string(), serde_json::Value::String("admin".to_string()));
let claims = jwt.create_standard_claims(Some(custom_claims));

// Sign the claims
let token = jwt.sign(&claims).unwrap();

// Verify the token
let mut verifier = jwt;
let verified_claims: StandardClaims = verifier.verify(&token).unwrap();
println!("Token verified: {}", verified_claims.sub);
```

### Direct Transition Methods

The builder provides convenient direct transition methods that validate and
transition states internally:

```rust
use agntcy_slim_auth::jwt::Jwt;
use agntcy_slim_auth::traits::{Signer, Verifier};
use jsonwebtoken_aws_lc::Algorithm;

// Using direct transition methods
let jwt = Jwt::builder()
    .issuer("https://mycompany.com")
    .audience("api-users")
    .subject("auth")
    .private_key(Algorithm::HS256, "your-secret-key").unwrap()  // Direct transition
    .build()
    .unwrap();
```

### Auto Key Resolution

The library supports automatic key resolution from OpenID Connect providers,
allowing you to verify tokens from external identity providers like Auth0, Azure
AD, or Okta.

```rust
use agntcy_slim_auth::jwt::{Jwt, StandardClaims};
use agntcy_slim_auth::traits::{AsyncVerifier, Verifier};
use std::time::Duration;

// Create a JWT instance with auto key resolution
let jwt = Jwt::builder()
    .issuer("https://your-tenant.auth0.com/")
    .audience("your-api")
    .subject("auth")
    .auto_resolve_keys(true).unwrap() // Enable automatic key resolution with direct transition
    .build()
    .unwrap();

// Verify a token asynchronously
let token = "..."; // Token received from the client
let mut verifier = jwt;
let claims: StandardClaims = verifier.verify_async(token).await.unwrap();
```

### OpenID Connect Discovery

The key resolver will first attempt to discover the JWKS URI using the standard
OpenID Connect discovery endpoint (`/.well-known/openid-configuration`) and will
fall back to the standard JWKS location (`/.well-known/jwks.json`) if needed.

### Custom Key Resolver

You can create a custom key resolver with specific settings:

```rust
use agntcy_slim_auth::jwt::Jwt;
use agntcy_slim_auth::resolver::KeyResolver;
use std::time::Duration;

// Create a custom key resolver
let resolver = KeyResolver::new()
    .with_ttl(Duration::from_secs(3600)); // 1 hour cache TTL

// Use it in a JWT instance
let jwt = Jwt::builder()
    .issuer("https://identity.mycompany.com")
    .audience("api")
    .subject("auth")
    .with_required_info().unwrap()
    .key_resolver(resolver)
    .build()
    .unwrap();
```

### Type-Safe State Machine Pattern

The builder uses a type state pattern to enforce the correct sequence of method
calls:

1. **Initial State**: Set basic properties (issuer, audience, subject)
2. **RequiredInfo State**: Transition after setting all required fields
3. **KeyConfig State**: Configure keys or auto-resolution
4. **Final State**: Ready to build the JWT

This ensures at compile-time that all required configuration is provided before
building the JWT object.

```rust
// Explicit state transitions
let jwt = Jwt::builder()
    .issuer("https://mycompany.com")
    .audience("api-users")
    .subject("auth")
    .with_required_info().unwrap()    // Transition to RequiredInfo state
    .private_key(Algorithm::HS256, "your-secret-key") // Transition to KeyConfig state
    .to_final_state().unwrap()        // Transition to Final state
    .build()                          // Build the JWT
    .unwrap();
```

## License

Apache-2.0
