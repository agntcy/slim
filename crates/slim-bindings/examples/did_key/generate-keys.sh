#!/usr/bin/env bash
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
#
# Generate the keys of the did:key examples.
#
#   generate-keys.sh <output-dir> <name>...
#
# For each <name> it writes <output-dir>/<name>.jwk, the app's Ed25519 private key as a
# JWK (RFC 8037), for the sender and receiver examples' --private-key. It also writes
# <output-dir>/trusted-keys.jwks, the JWKS with the public key of every <name>, for
# --trusted-keys. Apps that share that JWKS accept each other; an app may also be given a
# JWKS with only the keys it should accept.
#
# Needs openssl (3.x or later, for Ed25519) and base64.

set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "usage: $0 <output-dir> <name>..." >&2
  exit 1
fi

out="$1"
shift
mkdir -p "$out"
umask 077

# The last 32 bytes of a DER encoding, as base64url without padding.
tail32() { tail -c 32 | base64 | tr '+/' '-_' | tr -d '=\n'; }

public_jwks=""
for name in "$@"; do
  openssl genpkey -algorithm ed25519 -outform DER -out "$out/$name.der"
  # PKCS#8: the seed is the last 32 bytes. SPKI: so is the public key.
  d=$(tail32 < "$out/$name.der")
  x=$(openssl pkey -inform DER -in "$out/$name.der" -pubout -outform DER | tail32)
  rm -f "$out/$name.der"

  printf '{"kty":"OKP","crv":"Ed25519","d":"%s","x":"%s"}\n' "$d" "$x" > "$out/$name.jwk"
  public_jwks="${public_jwks:+$public_jwks,}{\"kty\":\"OKP\",\"crv\":\"Ed25519\",\"alg\":\"EdDSA\",\"use\":\"sig\",\"x\":\"$x\"}"
  echo "$name: $out/$name.jwk"
done

printf '{"keys":[%s]}\n' "$public_jwks" > "$out/trusted-keys.jwks"
echo "trust anchor: $out/trusted-keys.jwks"
