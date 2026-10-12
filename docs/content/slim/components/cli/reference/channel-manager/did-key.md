# slimctl channel-manager did-key

Print the did:key of an Ed25519 private key: the owner of the channels it creates, and how the Channel Manager's [`operators`](../../../channel-manager/config.md#operators) list it. It needs no Channel Manager.

## Usage

```
slimctl channel-manager did-key --key <PATH>
```

## Examples

```bash
openssl genpkey -algorithm ed25519 -out admin.pem
slimctl channel-manager did-key --key admin.pem
```

## Options

| Flag | Required | Description |
|------|----------|-------------|
| `--key <PATH>` | **Yes** | Ed25519 private key (PKCS#8 PEM) |
