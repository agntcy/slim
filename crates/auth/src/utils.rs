// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! General utility helpers for the authentication crate.

use base64::Engine;

/// Convert arbitrary bytes into a PEM-formatted string with the provided header/footer.
/// The body is wrapped at 64 character lines per RFC 7468 guidance.
/// Header/footer should include the BEGIN/END lines with trailing/leading newlines as desired.
pub fn bytes_to_pem(key_bytes: &[u8], header: &str, footer: &str) -> String {
    // Use base64 with standard encoding (not URL safe)
    let encoded = base64::engine::general_purpose::STANDARD.encode(key_bytes);

    // Insert newlines every 64 characters as per PEM format
    let mut pem_body = String::new();
    for i in 0..(encoded.len().div_ceil(64)) {
        let start = i * 64;
        let end = std::cmp::min(start + 64, encoded.len());
        if start < encoded.len() {
            pem_body.push_str(&encoded[start..end]);
            if end < encoded.len() {
                pem_body.push('\n');
            }
        }
    }

    format!("{}{}{}", header, pem_body, footer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytes_to_pem_basic() {
        let data = b"hello world"; // base64: aGVsbG8gd29ybGQ=
        let pem = bytes_to_pem(data, "-----BEGIN TEST-----\n", "\n-----END TEST-----");
        assert!(pem.starts_with("-----BEGIN TEST-----"));
        assert!(pem.ends_with("-----END TEST-----"));
        assert!(pem.contains("aGVsbG8gd29ybGQ="));
    }

    #[test]
    fn test_bytes_to_pem_line_wrapping() {
        // 70 bytes -> base64 longer than 64 chars to test wrapping
        let data = vec![b'A'; 70];
        let pem = bytes_to_pem(&data, "-----BEGIN TEST-----\n", "\n-----END TEST-----");
        // Count lines between header and footer
        let body = pem
            .replace("-----BEGIN TEST-----\n", "")
            .replace("\n-----END TEST-----", "");
        let lines: Vec<&str> = body.split('\n').collect();
        assert!(lines.len() >= 2, "Should have wrapped into multiple lines");
        assert!(lines.iter().all(|l| l.len() <= 64));
    }
}
