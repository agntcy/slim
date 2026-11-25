// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Throwaway TLS and JWT key material for tests.
//!
//! Nothing is committed to the repository: every directory returned here is
//! generated on first use, once per test process, under the OS temp dir. The
//! file names match the fixtures that used to live in `testdata/` so call sites
//! only change their base path.
//!
//! Directories are not removed when the process exits (statics are never
//! dropped); they only hold a few small files under the OS temp dir.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, PKCS_ECDSA_P384_SHA384, PKCS_ED25519, PKCS_RSA_SHA256,
    SanType,
};

/// Re-export so callers can pick leaf key algorithms without depending on `rcgen`.
pub use rcgen::PKCS_ECDSA_P256_SHA256 as ECDSA_P256_SHA256;
pub use rcgen::PKCS_RSA_SHA256 as RSA_SHA256;

/// DNS name carried by every generated server and client certificate.
pub const DNS_NAME: &str = "example1";

/// A certificate (PEM) and its private key (PEM).
#[derive(Clone, Debug)]
pub struct CertAndKey {
    pub cert: String,
    pub key: String,
}

/// A CA plus a server and a client certificate signed by it.
#[derive(Clone, Debug)]
pub struct TlsChain {
    pub ca: CertAndKey,
    pub server: CertAndKey,
    pub client: CertAndKey,
}

fn ca_params(common_name: &str) -> CertificateParams {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("ca params");
    params
        .distinguished_name
        .push(DnType::CommonName, common_name);
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params
}

fn leaf_params(common_name: &str, purpose: ExtendedKeyUsagePurpose) -> CertificateParams {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("leaf params");
    params
        .distinguished_name
        .push(DnType::CommonName, common_name);
    params.subject_alt_names = vec![
        SanType::DnsName(DNS_NAME.try_into().expect("dns name")),
        SanType::DnsName("localhost".try_into().expect("dns name")),
        SanType::IpAddress("127.0.0.1".parse().expect("ip")),
    ];
    params.extended_key_usages = vec![purpose];
    params
}

/// Generate a CA and a server and client certificate signed by it. The leaf
/// keys use `leaf_alg` (e.g. `&PKCS_RSA_SHA256`).
pub fn generate_chain(common_name: &str, leaf_alg: &'static rcgen::SignatureAlgorithm) -> TlsChain {
    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("ca key");
    let ca_cert = ca_params(common_name)
        .self_signed(&ca_key)
        .expect("ca cert");
    let issuer = Issuer::new(ca_params(common_name), ca_key);

    let leaf = |cn: &str, purpose| {
        let key = KeyPair::generate_for(leaf_alg).expect("leaf key");
        let cert = leaf_params(cn, purpose)
            .signed_by(&key, &issuer)
            .expect("leaf cert");
        CertAndKey {
            cert: cert.pem(),
            key: key.serialize_pem(),
        }
    };

    TlsChain {
        ca: CertAndKey {
            cert: ca_cert.pem(),
            key: issuer.key().serialize_pem(),
        },
        server: leaf("server", ExtendedKeyUsagePurpose::ServerAuth),
        client: leaf("client", ExtendedKeyUsagePurpose::ClientAuth),
    }
}

fn write(dir: &Path, file: &str, contents: &str) {
    fs::write(dir.join(file), contents).expect("write test key file");
}

/// Creates a fresh, randomly named directory under the system temp dir.
/// `create_dir` (not `create_dir_all`) fails if the name is already taken, so a
/// stale directory is never reused.
fn create_unique_dir() -> PathBuf {
    use std::hash::{BuildHasher, Hasher, RandomState};
    for _ in 0..16 {
        let suffix = RandomState::new().build_hasher().finish();
        let dir = std::env::temp_dir().join(format!("slim-test-keys-{suffix:016x}"));
        if fs::create_dir(&dir).is_ok() {
            return dir;
        }
    }
    panic!("could not create a unique test key dir");
}

fn sub_dir(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(&dir).expect("create test key dir");
    dir
}

fn generate_tls(dir: &Path) {
    for (n, alg) in [(1, &PKCS_ECDSA_P256_SHA256), (2, &PKCS_RSA_SHA256)] {
        let chain = generate_chain(&format!("slim-test-ca-{n}"), alg);
        write(dir, &format!("ca-{n}.crt"), &chain.ca.cert);
        write(dir, &format!("server-{n}.crt"), &chain.server.cert);
        write(dir, &format!("server-{n}.key"), &chain.server.key);
        write(dir, &format!("client-{n}.crt"), &chain.client.cert);
        write(dir, &format!("client-{n}.key"), &chain.client.key);
    }
    write(
        dir,
        "ca-bad.crt",
        "-----BEGIN CERTIFICATE-----\nbad certificate\n-----END CERTIFICATE-----\n",
    );
}

fn generate_grpc(dir: &Path) {
    let chain = generate_chain("slim-test-grpc-ca", &PKCS_ECDSA_P256_SHA256);
    write(dir, "ca.crt", &chain.ca.cert);
    write(dir, "server.crt", &chain.server.cert);
    write(dir, "server.key", &chain.server.key);
}

fn generate_jwt(dir: &Path) {
    let algs = [
        ("ec256", &PKCS_ECDSA_P256_SHA256),
        ("ec384", &PKCS_ECDSA_P384_SHA384),
        ("eddsa", &PKCS_ED25519),
        ("rsa", &PKCS_RSA_SHA256),
    ];
    for (name, alg) in algs {
        let key = KeyPair::generate_for(alg).expect("jwt key");
        write(dir, &format!("{name}.pem"), &key.serialize_pem());
        write(dir, &format!("{name}-public.pem"), &key.public_key_pem());
        let wrong = KeyPair::generate_for(alg).expect("wrong jwt key");
        write(dir, &format!("{name}-wrong.pem"), &wrong.serialize_pem());
    }
}

/// Root of the generated fixtures, with `tls/`, `grpc/` and `jwt/` below it.
/// Generated once per process.
///
/// - `tls/`: `ca-1.crt`, `ca-2.crt`, `ca-bad.crt` and
///   `{server,client}-{1,2}.{crt,key}`. Set 1 uses ECDSA P-256 leaf keys, set 2
///   RSA leaf keys; each set has its own CA. `ca-bad.crt` is not a valid
///   certificate. Leaves carry the SANs `example1`, `localhost`, `127.0.0.1`.
/// - `grpc/`: `ca.crt`, `server.crt`, `server.key`.
/// - `jwt/`: `{ec256,ec384,eddsa,rsa}.pem` (private keys), `*-public.pem`
///   (matching public keys) and `*-wrong.pem` (an unrelated private key of the
///   same type).
pub fn testdata_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = create_unique_dir();
        generate_tls(&sub_dir(&root, "tls"));
        generate_grpc(&sub_dir(&root, "grpc"));
        generate_jwt(&sub_dir(&root, "jwt"));
        root
    })
}

/// `testdata_dir()/tls`.
pub fn tls_dir() -> PathBuf {
    testdata_dir().join("tls")
}

/// `testdata_dir()/grpc`.
pub fn grpc_dir() -> PathBuf {
    testdata_dir().join("grpc")
}

/// `testdata_dir()/jwt`.
pub fn jwt_dir() -> PathBuf {
    testdata_dir().join("jwt")
}

/// `dir/file` as a `String`, for config structs that take paths as strings.
pub fn path_in(dir: &Path, file: &str) -> String {
    dir.join(file).to_string_lossy().into_owned()
}

/// The PKCS#8 PEM of the Ed25519 private key with the hex-encoded `seed`, as openssl
/// writes it.
pub fn ed25519_pem_of_seed(seed_hex: &str) -> String {
    use base64::Engine as _;

    let hex = format!("302e020100300506032b657004220420{seed_hex}");
    let der: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let body = base64::engine::general_purpose::STANDARD.encode(der);
    format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n")
}

/// A JWKS with the Ed25519 public keys `public_keys`, each given as base64url (the `x`
/// of its JWK).
pub fn ed25519_jwks_of(public_keys: &[&str]) -> String {
    let keys: Vec<String> = public_keys
        .iter()
        .map(|x| format!(r#"{{"kty":"OKP","crv":"Ed25519","alg":"EdDSA","use":"sig","x":"{x}"}}"#))
        .collect();
    format!(r#"{{"keys":[{}]}}"#, keys.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_contain_expected_files() {
        for f in [
            "ca-1.crt",
            "ca-2.crt",
            "ca-bad.crt",
            "server-2.key",
            "client-1.crt",
        ] {
            assert!(tls_dir().join(f).is_file(), "{f}");
        }
        for f in ["ca.crt", "server.crt", "server.key"] {
            assert!(grpc_dir().join(f).is_file(), "{f}");
        }
        for f in [
            "rsa.pem",
            "rsa-public.pem",
            "rsa-wrong.pem",
            "eddsa.pem",
            "ec384-wrong.pem",
        ] {
            assert!(jwt_dir().join(f).is_file(), "{f}");
        }
    }
}
