#!/usr/bin/env bash
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
#
# Generate the keys of the did:key examples.
#
#   generate-keys.sh <output-dir> <name>...
#
# For each <name> it writes two files in <output-dir>, each holding one line of base64:
#
#   <name>.private  the Ed25519 private key (a 48-byte PKCS#8 document), for the
#                   sender and receiver examples' --private-key
#   <name>.public   the matching public key (32 bytes), for the --trusted-key of the
#                   apps that accept <name>
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

for name in "$@"; do
  openssl genpkey -algorithm ed25519 -outform DER -out "$out/$name.der"
  base64 < "$out/$name.der" | tr -d '\n' > "$out/$name.private"
  # The public key is the last 32 bytes of its DER encoding.
  openssl pkey -inform DER -in "$out/$name.der" -pubout -outform DER \
    | tail -c 32 | base64 | tr -d '\n' > "$out/$name.public"
  rm -f "$out/$name.der"
  echo "$name: $out/$name.private $out/$name.public"
done
