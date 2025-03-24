#!/bin/bash
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0


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
openssl ecparam             \
    -genkey                 \
    -name secp384r1         \
    -out ${SERVER_CA_KEY}

# Server CA Cert
openssl req                 \
    -x509                   \
    -new                    \
    -key ${SERVER_CA_KEY}   \
    -out ${SERVER_CA_CERT}  \
    -config ${CA_CNF}

# Server Key
openssl ecparam     \
    -genkey         \
    -name secp384r1 \
    -out ${SERVER_KEY}

# Server CSR
openssl req                 \
    -new                    \
    -key ${SERVER_KEY}      \
    -out ${SERVER_CSR}      \
    -config ${SERVER_CNF}

# Server Cert
openssl x509                \
    -req                    \
    -in ${SERVER_CSR}       \
    -CA ${SERVER_CA_CERT}   \
    -CAkey ${SERVER_CA_KEY} \
    -CAcreateserial         \
    -out ${SERVER_CERT}     \
    -extfile ${SERVER_CNF}  \
    -extensions req_ext

# Client CA Key
openssl ecparam             \
    -genkey                 \
    -name secp384r1         \
    -out ${CLIENT_CA_KEY}

# Client CA Cert
openssl req                 \
    -x509                   \
    -new                    \
    -key ${CLIENT_CA_KEY}   \
    -out ${CLIENT_CA_CERT}  \
    -config ${CA_CNF}

# Client Key
openssl ecparam     \
    -genkey         \
    -name secp384r1 \
    -out ${CLIENT_KEY}

# Client CSR
openssl req                 \
    -new                    \
    -key ${CLIENT_KEY}      \
    -out ${CLIENT_CSR}      \
    -config ${CLIENT_CNF}

# Client Cert
openssl x509                \
    -req                    \
    -in ${CLIENT_CSR}       \
    -CA ${CLIENT_CA_CERT}   \
    -CAkey ${CLIENT_CA_KEY} \
    -CAcreateserial         \
    -out ${CLIENT_CERT}     \
    -extfile ${CLIENT_CNF}  \
    -extensions req_ext
