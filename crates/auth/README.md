# AGNTCY Slim Auth

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

This crate provides authentication and authorization capabilities for Agntcy SLIM,
with a focus on JWT (JSON Web Token) authentication and SPIFFE/SPIRE integration.

## Features

- JWT token creation and verification
- Builder pattern for fluent JWT configuration
- Flexible key resolution for JWT verification
- Support for OpenID Connect Discovery
- JWKS (JSON Web Key Set) integration
- Asynchronous verification for improved performance
- **SPIFFE/SPIRE integration for zero-trust workload identity**
  - X.509 SVID automatic rotation
  - JWT SVID with configurable audiences
  - Native Workload API integration
  - Support for federated trust domains

## Testing

This crate includes comprehensive unit tests and integration tests:

- **Unit tests**: Test individual components and error handling
- **Integration tests**: Test real interactions with SPIRE server and agent using Docker containers

### Running Tests

```bash
# Run unit tests only
cargo test --lib

# Run integration tests (requires Docker)
cargo test --test spiffe_integration_test -- --ignored --nocapture

# Run all tests
cargo test -- --include-ignored
```
