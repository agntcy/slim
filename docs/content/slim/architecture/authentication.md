# Authentication

SLIM authentication has two distinct concerns that operate independently: **transport security** between network peers, and **application identity** used to authenticate messages between applications.

## Transport Security (TLS / mTLS)

Connections between clients and SLIM nodes, and between SLIM nodes themselves, can be secured with TLS. This encrypts traffic in transit and, when mutual TLS (mTLS) is used, provides cryptographic authentication at the connection level.

- **TLS**: encrypts traffic; clients verify the server certificate against a trusted CA.
- **mTLS**: both sides present and verify certificates, providing mutual connection-level authentication.

Transport security is configured on the SLIM Data Plane and applies to connections independently of how applications identify themselves within those connections.

## Application Identity

Every SLIM application has an identity — a string identifier that travels with every message the application sends. The form of this identifier depends on the credential method in use:

| Credential method | Identity |
|-------------------|----------|
| Shared secret | `<base_name>_<random_suffix>` — derived from the application name with a unique random component |
| JWT | The `sub` (subject) claim of the JWT, set by the token issuer |
| SPIRE | The SPIFFE ID from the JWT-SVID (e.g. `spiffe://domain.test/ns/default/sa/my-app`) |

The credential does not authenticate the application *to the SLIM node* as in a login model. It determines how the application's identity token is **signed when sending** and **verified when receiving**. The receiving session validates the sender's identity token using the same credential mechanism; messages whose identity cannot be verified are dropped.

## Credential Methods

### Shared Secret

A symmetric key used to sign and verify application identity tokens. Each application's identity is independently generated from a base name (derived from the application name) plus a random suffix. When the application sends a message, SLIM creates a token containing the identity and signs it with an HMAC keyed by the shared secret. When a session receives a message, it verifies the HMAC using the same secret — if verification passes, the sender's identity is trusted; if not, the message is dropped.

**Properties:**
- Simple to configure — no external infrastructure required
- Any application holding the same secret can verify messages from any other holder
- Each application has a unique identity (base name + random suffix); the secret only governs whether that identity is trusted

**Use when:** Development, local testing, or closed internal networks where all participants are equally trusted.

### JWT (JSON Web Tokens)

The application holds a signed JWT. The `sub` (subject) claim of the JWT is the application's identity. When the application sends a message, the JWT is included as the identity token. When a session receives a message, it verifies the JWT signature using the issuer's public key or JWKS endpoint — if valid, the `sub` claim is accepted as the sender's identity.

Tokens can come from:

- **An external identity provider** — any OIDC-compatible IdP issues tokens; SLIM verifies them against the provider's public key or JWKS endpoint.
- **SLIM-issued tokens** — a SLIM node can sign and issue tokens directly if provided with a private signing key, useful when no external IdP is available.

**Properties:**
- Tokens are short-lived and can be revoked at expiry
- Each application has a distinct identity tied to the `sub` claim set by the issuer
- Stateless verification — receivers only need the issuer's public key

**Use when:** Service-to-service authentication with an existing identity provider, or when per-application credential differentiation is needed without a full PKI.

### SPIRE / SPIFFE

[SPIFFE](https://spiffe.io/) (Secure Production Identity Framework For Everyone) is a standard for workload identity. [SPIRE](https://spiffe.io/docs/latest/spire-about/) is its reference implementation, issuing JWT-SVIDs to workloads via a local agent socket.

SLIM integrates with SPIRE by consuming JWT-SVIDs from the SPIRE Workload API. The SPIFFE ID embedded in the JWT-SVID (e.g. `spiffe://domain.test/ns/default/sa/my-app`) is the application's identity. When a session receives a message, it validates the JWT-SVID against the SPIRE trust bundle (JWT bundle set) — if valid, the SPIFFE ID is accepted as the sender's identity.

**Properties:**
- **Zero-secret bootstrapping** — no static secrets or certificates need to be distributed to workloads
- **Automatic rotation** — SPIRE rotates JWT-SVIDs before expiry; applications receive fresh credentials without restart
- **Workload attestation** — SPIRE verifies the identity of the requesting workload (Kubernetes ServiceAccount, process attributes, etc.) before issuing credentials, making impersonation very difficult

**Use when:** Production deployments, especially Kubernetes, where strong workload identity and automatic rotation are required.

## Choosing a Credential Method

| Method | Application identity | Rotation | Workload attestation | Recommended for |
|--------|---------------------|---------|---------------------|-----------------|
| Shared secret | Base name + random suffix | Manual | No | Development, trusted LANs |
| JWT (external IdP) | `sub` claim from token | Token expiry | IdP-dependent | Existing IdP integration |
| JWT (SLIM-issued) | `sub` claim from token | Token expiry | No | Simple production, no IdP |
| SPIRE | SPIFFE ID | Automatic | Yes | Production, Kubernetes |

## Related

- [Naming](./naming.md) — How SLIM names map to application identities
- [SLIM Data Plane Configuration](../components/data-plane/config.md) — Configure TLS, mTLS, shared secret, and JWT on a SLIM node
- [Kubernetes Deployment](../deploy/kubernetes.md) — Deploy SLIM with SPIRE in Kubernetes
