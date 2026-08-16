# Configuring slimctl for Persistent TLS Connection

To permanently configure slimctl to connect to your SLIM node at `myhost.internal:9090` with mTLS using your certificates, run the following commands:

```bash
slimctl config set server myhost.internal:9090
slimctl config set tls-insecure false
slimctl config set tls-ca-file /etc/pki/ca.pem
slimctl config set tls-cert-file /etc/pki/client.pem
slimctl config set tls-key-file /etc/pki/client.key
```

**Important:** The `tls-insecure false` setting is critical. By default, slimctl sets `tls-insecure` to `true`, which means it uses plain HTTP/2 with no TLS. Even if you provide cert and CA file paths, they will be ignored unless you explicitly set `tls-insecure` to `false`. This command disables the insecure mode and enables TLS on the connection.

These settings are persisted in `~/.slimctl/config.yaml`, so every subsequent `slimctl` invocation will automatically use this server address and TLS configuration without needing to pass flags each time.

You can verify your configuration at any time with:

```bash
slimctl config list
```
