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
- **OIDC identity bound to the MLS signing key via DPoP (RFC 9449)**
  - Maps an SSO user onto the MLS credential, with no custom claim mapper
  - RFC 7638 JWK thumbprints for `cnf.jkt` verification
  - Authorization Code + PKCE and refresh-token grants, both DPoP-proofed

## OIDC with DPoP

`OidcTokenProvider` / `OidcVerifier` make the MLS identity the identity provider's
`sub` — the signed-in person — instead of a value minted per application instance.
See [Authentication](../../docs/content/slim/architecture/authentication.md) for
the user-facing description; this section covers the crate internals.

**Requires an RFC 9449 provider** (Keycloak 24+, with "OAuth 2.0 DPoP" enabled).
Supported key types are those with a JOSE mapping: P-256 (`ES256`) and Ed25519
(`EdDSA`).

### Ciphersuite agreement

`generate_mls_signature_keys` picks its curve from the **compile-time**
`curve25519` feature, while the MLS ciphersuite can also be chosen at **runtime**
via `enforce_pqc`. The two must agree, and `ML_KEM_768_X25519` signs with
Ed25519 — only its KEM is post-quantum — so a post-quantum deployment needs the
`curve25519` feature enabled. `Mls::build_client` derives the public key from the
installed secret using the active ciphersuite and refuses a pair that does not
match, so a mismatch fails at startup instead of inside a later group operation.

### Module map

| Item | Purpose |
|------|---------|
| `dpop::build_proof` | Construct the `dpop+jwt` proof sent to the token endpoint |
| `dpop::jwk_thumbprint` | RFC 7638 thumbprint of an MLS public key — the `cnf.jkt` value |
| `oidc::post_token_request_with_dpop` | Token-endpoint POST carrying a proof, with the RFC 9449 §8 nonce retry |
| `OidcTokenProvider::exchange_authorization_code` | Authorization-code grant; mints the binding |
| `OidcTokenProvider::install_signature_keys` | Adopt a key pair minted elsewhere (e.g. by `slimctl login --dpop`) |

`dpop` is free of `jsonwebtoken` and `reqwest` so it also compiles for `wasm32`,
where thumbprints are still needed but those crates are unavailable.

### How the key reaches the verifier

A DPoP token commits to the key only as a one-way hash, but verification needs
the key itself. The holder therefore presents it alongside the token, in the
credential `get_token()` returns:

```text
<access token>~<base64url MLS public key>
```

`OidcVerifier` splits this, validates the token via JWKS, re-hashes the presented
key, and only if it equals `cnf.jkt` surfaces it as an ordinary `pubkey` claim.
Everything downstream — `IdentityClaims`, MLS `validate_member`, e2e header
signature checks — then reads a normal raw-public-key claim and needs no DPoP
awareness.

This rides in the existing `SLIMHeader.identity` field, which is already an
opaque provider-specific string (`SharedSecret` puts a colon-delimited MAC format
there, not a JWT). No wire-format change was required — which matters, because
`prost` drops unknown fields and relay nodes re-encode forwarded messages, so a
new proto field would be silently stripped by any node running an older build.

A credential with no `~` is a plain bearer token and takes the unchanged path, so
transport-auth use of `OidcVerifier` is unaffected.

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
