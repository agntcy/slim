# slimctl Privacy Policy

`slimctl` is the command-line tool for SLIM nodes, the SLIM controller and the
channel manager. AGNTCY runs no service behind it: `slimctl` has no telemetry
and sends nothing to AGNTCY. It talks only to the endpoints and identity
provider you configure, and what they do with the data is up to whoever
operates them.

## What it reads locally

- Command-line flags, and environment variables prefixed `SLIMCTL_`.
- A config file: the one passed with `--config` (or `SLIMCTL_CONFIG`), else
  `~/.slimctl/config.yaml`, else `./config.yaml`. On Windows `~` is
  `%USERPROFILE%`. `${env:VAR}` and `${file:/path}` references in it are
  expanded when it is loaded.
- `~/.slimctl/token`, the bearer token written by `slimctl login`.
- Files you name: TLS CA, client certificate and key files, channel grants
  and signed requests (or stdin), Ed25519 private keys passed to
  `slimctl channel-manager` with `--key`, and node configs for
  `slimctl slim start`.

## What it writes locally

- `slimctl login` writes to `~/.slimctl/`:
  - `credentials.yaml`: the ID token, access token and refresh token from your
    identity provider, plus its client ID, issuer and token endpoint. The ID
    token carries whatever claims the provider releases for the scopes
    `openid profile email offline_access groups`, typically your name, email
    address and group memberships.
  - `token`: the access token, or the ID token if no access token was issued.
  - `refresh_token`, when one was issued. It is there for long-running SLIM
    processes to use; `slimctl` itself never uses it.
- `slimctl login` and `slimctl config set` update the config file. `login`
  records the endpoint and an auth block that points at `~/.slimctl/token`,
  never the token itself. `config set basic-auth-creds` stores the username
  and password in the file. To keep the password out of it, reference it
  with `${env:VAR}` or `${file:/path}` instead.
- `slimctl slim start` without `--config` writes a temporary node config,
  holding only the listen address, and deletes it on exit.

On Linux and macOS, `slimctl` writes all of these files readable only by you
(mode 0600), and narrows any that already existed with wider permissions. On
Windows they inherit the permissions of your user profile folder.

## What it sends, and to whom

- **Node, controller and channel-manager commands** connect to the endpoint
  you configure with `--server` or the config file. Without one, they use
  loopback addresses on your own machine. They send the request data you give
  (node, route, link, channel and member names, and grants), plus your
  credentials: a basic-auth username and password, a bearer token, or a TLS
  client certificate. A `host:port` with no scheme and no TLS settings is
  reached over plain HTTP, so credentials sent to it are unencrypted. Use
  `https://` or the TLS options for remote endpoints.
- **Channel-manager commands given `--key`** sign the request on your
  machine with that Ed25519 private key, and send the signature and the
  key's public did:key. The private key itself is never sent anywhere.
- **`slimctl login`** talks only to the OpenID Connect provider you name with
  `--discovery-uri`:
  - It fetches the discovery document and the signing keys, all over HTTPS.
  - It opens your browser at the provider's sign-in page; on Windows it
    prints the URL for you to open.
  - It receives the sign-in result on a loopback address
    (`http://127.0.0.1:8250/callback` by default).
  - It exchanges that result at the provider's token endpoint, sending the
    client ID and a one-time PKCE verifier.
- **`slimctl bench`** connects to the SLIM node you name and sends synthetic
  messages, authenticated with the shared secret you pass.
- **`slimctl slim start`** runs a SLIM node on your machine. It listens on
  `127.0.0.1:46357` by default, without TLS, and forwards messages between the
  applications that connect to it. With your own config it does what that
  config says, including exporting OpenTelemetry data if you enable it there.

## Credentials in output

Credentials are held in memory only while a command runs. `slimctl login`
prints the paths it wrote, not the tokens. `slimctl config list` shows
passwords, client secrets, refresh tokens, header values and inline private
keys as `(redacted)`, including values it expanded from `${env:...}` or
`${file:...}` references.

## Telemetry

`slimctl` collects no usage analytics, crash reports or other telemetry, and
makes no network calls beyond the ones described above.

## Changes to this policy

This file is versioned with `slimctl` in the agntcy/slim repository; its git
history records every change.

## Contact

Open an issue at <https://github.com/agntcy/slim/issues>. Report security
vulnerabilities as described in
[SECURITY.md](https://github.com/agntcy/slim/blob/main/SECURITY.md).
