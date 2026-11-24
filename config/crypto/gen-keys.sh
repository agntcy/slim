#!/bin/bash
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Generates the demo TLS keys/certs and JWT keys used by the example configs in
# `config/`. Nothing generated here is committed (see .gitignore): run this
# script, or `task config:keys`, before starting an example.
#
# Requires OpenSSL >= 1.1.1 (LibreSSL, the macOS default, is not supported:
# it lacks ED25519 and writes v1 CA certificates). On macOS: `brew install
# openssl` and put it first on PATH, or point OPENSSL at it.

set -euo pipefail

OPENSSL="${OPENSSL:-openssl}"
if ! version="$("$OPENSSL" version 2>/dev/null)" || [[ "$version" != OpenSSL\ * ]]; then
    echo "gen-keys.sh: OpenSSL >= 1.1.1 is required, found: ${version:-none}" >&2
    echo "On macOS: brew install openssl, then re-run with OPENSSL=\$(brew --prefix openssl)/bin/openssl" >&2
    exit 1
fi
ver_num="${version#OpenSSL }"
ver_num="${ver_num%% *}"
if [[ "$(printf '%s\n1.1.1\n' "$ver_num" | sort -V | head -n1)" != "1.1.1" ]]; then
    echo "gen-keys.sh: OpenSSL >= 1.1.1 is required, found: $version" >&2
    exit 1
fi

# The .cnf files and the output location are relative to this script.
cd "$(dirname "$0")"

CA_CNF="ca.cnf"

SERVER_CNF="server.cnf"
SERVER_CA_KEY="server-ca-key.pem"
SERVER_CA_CERT="server-ca-cert.pem"
SERVER_CSR="server-csr.pem"
SERVER_CERT="server-cert.pem"
SERVER_KEY="server-key.pem"

CLIENT_CNF="client.cnf"
CLIENT_CA_KEY="client-ca-key.pem"
CLIENT_CA_CERT="client-ca-cert.pem"
CLIENT_CSR="client-csr.pem"
CLIENT_CERT="client-cert.pem"
CLIENT_KEY="client-key.pem"

# Server CA Key
"$OPENSSL" ecparam             \
    -genkey                 \
    -name secp384r1         \
    -out ${SERVER_CA_KEY}

# Server CA Cert
"$OPENSSL" req                 \
    -x509                   \
    -new                    \
    -key ${SERVER_CA_KEY}   \
    -out ${SERVER_CA_CERT}  \
    -config ${CA_CNF}       \
    -days 3650

# Server Key
"$OPENSSL" ecparam     \
    -genkey         \
    -name secp384r1 \
    -out ${SERVER_KEY}

# Server CSR
"$OPENSSL" req                 \
    -new                    \
    -key ${SERVER_KEY}      \
    -out ${SERVER_CSR}      \
    -config ${SERVER_CNF}

# Server Cert
"$OPENSSL" x509                \
    -req                    \
    -in ${SERVER_CSR}       \
    -CA ${SERVER_CA_CERT}   \
    -CAkey ${SERVER_CA_KEY} \
    -CAcreateserial         \
    -out ${SERVER_CERT}     \
    -extfile ${SERVER_CNF}  \
    -extensions req_ext     \
    -days 3650

# Client CA Key
"$OPENSSL" ecparam             \
    -genkey                 \
    -name secp384r1         \
    -out ${CLIENT_CA_KEY}

# Client CA Cert
"$OPENSSL" req                 \
    -x509                   \
    -new                    \
    -key ${CLIENT_CA_KEY}   \
    -out ${CLIENT_CA_CERT}  \
    -config ${CA_CNF}       \
    -days 3650

# Client Key
"$OPENSSL" ecparam     \
    -genkey         \
    -name secp384r1 \
    -out ${CLIENT_KEY}

# Client CSR
"$OPENSSL" req                 \
    -new                    \
    -key ${CLIENT_KEY}      \
    -out ${CLIENT_CSR}      \
    -config ${CLIENT_CNF}

# Client Cert
"$OPENSSL" x509                \
    -req                    \
    -in ${CLIENT_CSR}       \
    -CA ${CLIENT_CA_CERT}   \
    -CAkey ${CLIENT_CA_KEY} \
    -CAcreateserial         \
    -out ${CLIENT_CERT}     \
    -extfile ${CLIENT_CNF}  \
    -extensions req_ext     \
    -days 3650

# Intermediate files are not needed once the certs are issued.
rm -f ${SERVER_CSR} ${CLIENT_CSR} ./*.srl

# JWT keys (private key + matching public key), one per supported family.
mkdir -p jwt

gen_jwt_key() {
    local name=$1
    shift
    "$OPENSSL" genpkey "$@" -out "jwt/${name}.pem"
    "$OPENSSL" pkey -in "jwt/${name}.pem" -pubout -out "jwt/${name}-public.pem"
}

gen_jwt_key ec256 -algorithm EC -pkeyopt ec_paramgen_curve:P-256
gen_jwt_key ec384 -algorithm EC -pkeyopt ec_paramgen_curve:P-384
gen_jwt_key eddsa -algorithm ED25519
gen_jwt_key rsa   -algorithm RSA -pkeyopt rsa_keygen_bits:2048
