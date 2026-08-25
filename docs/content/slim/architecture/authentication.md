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
| OIDC with DPoP | The `sub` claim from your identity provider — the **person**, shared by every application instance they run |

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

### OIDC with DPoP

Every method above gives each application instance its own identity. This one does the opposite: it maps the identity a **person** already has in your SSO provider onto the applications they run, so a user's laptop and phone present the same identity rather than two unrelated ones.

The obstacle is that every other method needs the application's MLS signing public key inside the identity token, and an external identity provider will not put a SLIM-specific claim in the tokens it issues. Rather than requiring a custom claim mapper, SLIM uses [DPoP (RFC 9449)](https://datatracker.ietf.org/doc/html/rfc9449) — a standard the provider already implements.

**How the binding is made:**

1. A signing key pair is generated, and a **DPoP proof** — a short-lived JWT signed with that key — is sent with the token request.
2. The provider verifies the proof and issues an access token containing `cnf.jkt`: the SHA-256 thumbprint ([RFC 7638](https://datatracker.ietf.org/doc/html/rfc7638)) of that public key. In effect the token now reads *"this provider attests that user `sub` holds the key whose thumbprint is `jkt`."*
3. The application uses that key as its MLS signing key and presents the token as its identity.
4. A receiving peer validates the token against the provider's JWKS, then hashes the key the sender actually presented and checks it equals `cnf.jkt`. A stolen token replayed with a different key fails this check.

After the initial binding there is no per-message DPoP overhead — MLS's own leaf-node signatures, made with the same key, are the continuing proof of possession. Renewing the token reuses the same key, so `cnf.jkt` is unchanged and the identity survives rotation without disturbing group membership.

**Setting it up:**

```bash
# 1. In Keycloak: enable "OAuth 2.0 DPoP" on the realm or client. No custom
#    protocol mapper is needed. Requires Keycloak 24 or newer.

# 2. Sign in. This generates the MLS signing key, has the provider bind it,
#    and saves both to ~/.slimctl/credentials.yaml (mode 0600).
slimctl login --dpop \
  --client-id slim-app \
  --discovery-uri https://keycloak.example.com/realms/slim/.well-known/openid-configuration
```

The command fails with a clear message if the provider returns a token without `cnf.jkt`, which almost always means DPoP is not enabled for that client or realm.

Applications configured with the OIDC identity provider for the same issuer pick up the saved key and refresh token automatically, and from then on authenticate as the signed-in user.

**Properties:**

- **One identity per person, not per process** — every instance a user runs shares their `sub`
- **No custom claim mappers** — uses DPoP and JWK thumbprints, which compliant providers already support
- **Proof of possession** — the token is cryptographically bound to the signing key, so a stolen token alone is not enough to impersonate
- **Survives token rotation** — renewal re-proves the same key, leaving the binding intact

**Requirements and trade-offs:**

- Requires an RFC 9449 provider (Keycloak 24+, or equivalent)
- Supports MLS ciphersuites whose signing keys map to a JOSE algorithm: NIST P-256 (`ES256`) and Ed25519 (`EdDSA`)
- **Post-quantum (`enforce_pqc`) requires the `curve25519` build feature.** The `ML_KEM_768_X25519` ciphersuite is post-quantum in its *key exchange* only — it still signs with Ed25519, which DPoP handles. But signing keys are generated for a curve fixed at build time, defaulting to P-256, while `enforce_pqc` is chosen at runtime. Build with `curve25519` so the two agree; otherwise the mismatch is rejected at startup with a message naming the cause
- The signing key is stored on disk so it can outlive a single process. This is what makes one identity spannable across instances, but it also means `~/.slimctl/credentials.yaml` **is** the user's identity, not merely a token — protect it accordingly, and rotate by signing in again if it is exposed
- Interactive sign-in requires a browser. Headless services should use a different method until device-flow support lands

**Use when:** Applications act on behalf of a human whose identity already lives in an SSO provider, and you need to answer "which real person does this credential belong to."

## Choosing a Credential Method

| Method | Application identity | Rotation | Workload attestation | Recommended for |
|--------|---------------------|---------|---------------------|-----------------|
| Shared secret | Base name + random suffix | Manual | No | Development, trusted LANs |
| JWT (external IdP) | `sub` claim from token | Token expiry | IdP-dependent | Existing IdP integration |
| JWT (SLIM-issued) | `sub` claim from token | Token expiry | No | Simple production, no IdP |
| SPIRE | SPIFFE ID | Automatic | Yes | Production, Kubernetes |
| OIDC with DPoP | `sub` claim — the signed-in person | Token expiry, key unchanged | No (user attestation, not workload) | User-facing apps with existing SSO |

## Related

- [Naming](./naming.md) — How SLIM names map to application identities
- [SLIM Data Plane Configuration](../components/data-plane/config.md) — Configure TLS, mTLS, shared secret, and JWT on a SLIM node
- [Kubernetes Deployment](../deploy/kubernetes.md) — Deploy SLIM with SPIRE in Kubernetes
