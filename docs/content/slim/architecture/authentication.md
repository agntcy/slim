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

DPoP binds a token to a key at the moment the token is issued, and that binding can never be changed afterwards. MLS signing keys, meanwhile, are generated by the MLS layer and rotate on their own schedule. So there are **two keys**, and the login binds the one that does not move:

- an **identity key**, generated once by `slimctl login` and named by the provider in `cnf.jkt`. It is never used to sign MLS messages.
- the app's **MLS signing keys**, generated by the MLS layer as usual, as often as it likes.

**How the binding is made:**

1. `slimctl login` generates the identity key and sends a **DPoP proof** — a short-lived JWT signed with it — along with the token request.
2. The provider verifies the proof and issues an access token containing `cnf.jkt`: the SHA-256 thumbprint ([RFC 7638](https://datatracker.ietf.org/doc/html/rfc7638)) of that public key. In effect the token now reads *"this provider attests that user `sub` holds the key whose thumbprint is `jkt`."*
3. When the app's MLS layer generates a signing key, the identity key signs a short-lived **key attestation** naming it. The app presents the token and that attestation together as its identity.
4. A receiving peer validates the token against the provider's JWKS, checks the attestation was signed by the key whose thumbprint is `cnf.jkt`, and takes the MLS key from inside it. A stolen credential replayed under a different MLS key fails, because forging a new attestation needs the identity key's private half.

After the initial binding there is no per-message DPoP overhead — MLS's own leaf-node signatures are the continuing proof of possession. Renewing the token re-proves the same identity key, so `cnf.jkt` is unchanged; and rotating an MLS key just mints a new attestation, with no provider contact and no new login.

**Setting it up:**

```bash
# 1. In Keycloak: enable "OAuth 2.0 DPoP" on the realm or client. No custom
#    protocol mapper is needed. Officially supported from Keycloak 26.4
#    (experimental from 23.0, with --features=dpop).

# 2. Sign in once per app. Each login generates that app's identity key, has the
#    provider bind it, and saves both to the named store (mode 0600). Only the
#    first prompts: the IdP's SSO cookie makes the rest silent redirects.
slimctl login --dpop-credentials-file ~/.slim/laptop.yaml \
  --client-id slim-app \
  --discovery-uri https://keycloak.example.com/realms/slim/.well-known/openid-configuration
```

The command fails with a clear message if the provider returns a token without `cnf.jkt`, which almost always means DPoP is not enabled for that client or realm.

Point each app at its own store, either with `credentials_file` in its OIDC identity config or with `SLIM_CREDENTIALS_FILE`. There is no default — an app that names no store fails at startup rather than silently sharing another app's identity.

```yaml
identity:
  type: oidc
  issuer_url: https://keycloak.example.com/realms/slim
  client_id: slim-app
  audience: slim
  credentials_file: ~/.slim/laptop.yaml
```

RFC 9449 binds a refresh token to the key from its original grant, so a distinct key means a distinct login. That is one browser round-trip per app, but only one credential prompt.

**Identity is not the same credential as the connection.** An app has two: its MLS identity, from a `--dpop-credentials-file` store, and whatever authenticates its gRPC connection to the node. They are configured separately and need not name the same principal.

A DPoP-bound token cannot serve as the connection credential: every refresh must carry a proof, and the transport provider holds no signing key, so it would connect fine and die at the first renewal. Transport auth therefore takes its own explicit credential — `client_secret` (client-credentials, no browser), `refresh_token_file`, or `refresh_token`. It does **not** read any stored login: a node that fell back to `~/.slimctl/credentials.yaml` authenticated as whoever last signed in on that host.

```bash
# app identity — DPoP-bound, one store per app, browser login
slimctl login --dpop-credentials-file ~/.slim/laptop.yaml --client-id slim-app --discovery-uri ...
slimctl login --dpop-credentials-file ~/.slim/ci.yaml     --client-id slim-app --discovery-uri ...
```

```yaml
# connection credential — a service identity, named explicitly
node:
  endpoint: "https://slim.example.com:46357"
  auth:
    type: oidc
    issuer_url: https://keycloak.example.com/realms/slim
    client_id: slim-transport
    client_secret: "${file:/run/secrets/slim-transport-secret}"
```

For the same reason, an OIDC **identity** cannot use `client_secret`: the client-credentials grant is not DPoP-bound, so its token carries no MLS key and peers reject it with `PublicKeyNotFound`. A headless workload wanting per-workload identity should use SPIRE.

**Properties:**

- **One identity per person, one key per app** — every store carries the same `sub`, and each app attests its own MLS key. Group membership is keyed on `{sub}:{public_key}`, so removing one app from a group leaves that person's other apps untouched
- **MLS keys stay MLS's** — rotation, restoring a persisted group, and per-session keys all work without a new login, because the provider never sees an MLS key
- **No custom claim mappers** — uses DPoP and JWK thumbprints, which compliant providers already support
- **Proof of possession** — the token is cryptographically bound to the identity key, and the MLS key to that, so a stolen credential alone is not enough to impersonate
- **Survives token rotation** — renewal re-proves the same identity key, leaving the binding intact

**Requirements and trade-offs:**

- Requires an RFC 9449 provider (Keycloak 26.4+ for official DPoP support, or equivalent)
- Works with any MLS ciphersuite. The identity key is always P-256 (`ES256`), and MLS signing keys are carried inside the attestation as opaque bytes, so they need no JOSE mapping of their own
- **Post-quantum (`enforce_pqc`) still requires the `curve25519` build feature.** Unchanged by the identity path, and not a DPoP constraint: MLS signing keys are generated for a curve fixed at build time, defaulting to P-256, while `enforce_pqc` selects `ML_KEM_768_X25519` — post-quantum in its *key exchange* only, still signing with Ed25519 — at runtime. Build with `curve25519` so the two agree; otherwise the mismatch is rejected at startup with a message naming the cause
- **Do not enable standard token exchange (RFC 8693) on the client used for MLS identity, and keep that client public.** The credential is published into the MLS group, where every member can read it. Measured on Keycloak 26.4, token exchange re-binds even a DPoP-bound token to whatever key the request proofs while preserving `sub` — contrary to Keycloak's documentation — so a member holding a peer's credential and the app's client credentials could impersonate that peer. Public clients cannot exchange at all, which is why the default configuration is safe; the risk is a later realm change
- The identity key is stored on disk so it can outlive a single process. This is what makes one identity spannable across instances, but it also means each credentials store **is** an app's identity, not merely a token — protect it accordingly, and re-run the login if it is exposed. Note the MLS private keys are *not* in the store: they live wherever the MLS layer keeps them
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
