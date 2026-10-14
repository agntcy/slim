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

### Name ids and identity binding

An application's name id, the last component of its name, is the XXH3-128 hash of its identity, and a multicast channel's ids are derived from its moderator's identity. Sessions check these against the identities they verify:

- **Control messages:** every control message is verified, with or without MLS, and its sender's name id must be the one its verified identity derives.
- **Payloads:** a control message's payload and metadata are signed too, with the key that signs its header, except trace context that nodes add in transit. A payload that doesn't match its signature is always refused; one without a signature comes from an older peer.
- **Invites:** a moderator accepts replies only from the name it invited, and the join reply must name its sender and, with MLS, carry a key package for that same identity. An invitee joins only a channel derived from the moderator's verified identity.
- **Membership:** a participant accepts group updates, removals, welcomes, the group's close and further invites only from its moderator, and a member may announce only its own state.
- **Data messages:** with MLS, a message's source name id must be the one its MLS sender's identity derives, so a group member can't send under another member's name.

The `SLIM_IDENTITY_BINDING` environment variable sets what happens on a mismatch: `warn` (the default) logs it, `enforce` drops the message, and `off` turns the checks off. Before setting `enforce`, check the logs for mismatches from applications whose identity provider can't report an id, and from channels that aren't created by their moderator.

A caller can also require a specific identity. `App::create_session_pinned`, for a point-to-point session, and `SessionController::invite_participant_pinned`, for a group member, admit the peer only if it is verified as the given `sub`, which is its DID for a `did:key` identity. A peer verified as anyone else is refused, whatever `SLIM_IDENTITY_BINDING` is set to. With MLS, `SessionController::peer_identity` and `participant_identity` return the identity each peer was verified as.

## Credential Methods

### Shared Secret

A symmetric key used to sign and verify application identity tokens. Each application's identity is independently generated from a base name (derived from the application name) plus a random suffix. When the application sends a message, SLIM creates a token containing the identity and signs it with an HMAC keyed by the shared secret. When a session receives a message, it verifies the HMAC using the same secret — if verification passes, the sender's identity is trusted; if not, the message is dropped.

**Properties:**
- Simple to configure — no external infrastructure required
- Any application holding the same secret can verify messages from any other holder — and, symmetrically, can *sign* as any identity that shares the secret. There is no per-application isolation and no non-repudiation: the secret proves membership in the group, not which member sent a given message.
- Each application has a unique identity (base name + random suffix); the secret only governs whether that identity is trusted
- A single leaked secret compromises every identity that uses it, and rotation means redistributing the new secret to every holder at once

**Use when:** Development, local testing, or closed internal networks where all participants are equally trusted. Prefer JWT or SPIRE for anything else — both give each application its own credential, so one compromised application does not let an attacker impersonate the rest.

### JWT (JSON Web Tokens)

The application holds a signed JWT. The `sub` (subject) claim of the JWT is the application's identity. When the application sends a message, the JWT is included as the identity token. When a session receives a message, it verifies the JWT signature using the issuer's public key or JWKS endpoint — if valid, the `sub` claim is accepted as the sender's identity.

Tokens can come from:

- **An external identity provider** — any OIDC-compatible IdP issues tokens; SLIM verifies them against the provider's public key or JWKS endpoint.
- **SLIM-issued tokens** — a SLIM node can sign and issue tokens directly if provided with a private signing key, useful when no external IdP is available.

With a trust list of peer keys, such as a JWKS of `did:key` identities, every listed key can sign a valid token. SLIM therefore also checks that a token's `sub` belongs to the key that signed it: it must be that key's `kid` or, for an Ed25519 key, its `did:key`. The `jwt` app identity always runs this check, in the mode set by the `SLIM_IDENTITY_BINDING` environment variable: `warn` (the default) logs a mismatch, `enforce` rejects the token, and `off` turns the check off. A JWT verifier configured by hand runs it when `subject_binding` is set. Tokens from an identity provider's keys aren't checked, because the provider vouches for each `sub`.

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
