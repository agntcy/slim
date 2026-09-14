To configure slimctl to always connect to your SLIM node at `myhost.internal:9090` using mTLS with your specified certificates, you need to persist these settings in the slimctl configuration file. Run the following `slimctl config set` commands:

```bash
# Set the server address
slimctl config set server myhost.internal:9090

# Disable insecure mode (plain HTTP/2) since you are using real TLS
slimctl config set tls-insecure false

# Set the CA certificate for verifying the server
slimctl config set tls-ca-file /etc/pki/ca.pem

# Set the client certificate for mutual TLS
slimctl config set tls-cert-file /etc/pki/client.pem

# Set the client private key
slimctl config set tls-key-file /etc/pki/client.key
```

These settings are persisted to `~/.slimctl/config.yaml` and will be used for every subsequent slimctl command. The resulting config file will look like this:

```yaml
common_opts:
  server: "myhost.internal:9090"
  timeout: "15s"
  tls_insecure: false
  tls_insecure_skip_verify: false
  tls_ca_file: "/etc/pki/ca.pem"
  tls_cert_file: "/etc/pki/client.pem"
  tls_key_file: "/etc/pki/client.key"
```

After running these commands, all slimctl operations (e.g., `slimctl node route list`, `slimctl controller node list`) will automatically connect to `myhost.internal:9090` over TLS using your CA and client certificates without needing to pass flags each time.

You can verify the configuration at any time with:

```bash
slimctl config list
```
