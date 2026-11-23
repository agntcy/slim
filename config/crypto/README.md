# Demo keys and certificates

Everything in this directory is for local demos and tests only. The private
keys are public, so never use them in a real deployment.

- `server-*.pem`, `client-*.pem`: EC P-384 CAs and a `localhost` server and
  client certificate, valid until 2035. Regenerate them with `gen-keys.sh`,
  run from this directory.
- `jwt/`: EC P-256, EC P-384, Ed25519 and RSA key pairs for the JWT examples in
  `config/`.

The same P-256 key as `jwt/ec256.pem` is inlined in
`config/jwt-auth-ecdsa/client-config.yaml`.
