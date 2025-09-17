// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Once;

static RUSTLS: Once = Once::new();

pub fn initialize_crypto_provider() {
    RUSTLS.call_once(|| {
        // check whether a default provider is already set
        if rustls::crypto::CryptoProvider::get_default().is_some() {
            return;
        }

        // Set aws-lc as default crypto provider
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .expect("Failed to install default crypto provider");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    /// Test that initialize_crypto_provider can be called successfully
    #[test]
    fn test_initialize_crypto_provider_success() {
        // The function should not panic or return an error
        initialize_crypto_provider();

        // Verify that a crypto provider is installed by testing random generation
        let result = rustls::crypto::CryptoProvider::get_default()
            .unwrap()
            .secure_random
            .fill(&mut [0u8; 32]);
        assert!(result.is_ok());
    }

    /// Test that calling initialize_crypto_provider multiple times is safe
    #[test]
    fn test_initialize_crypto_provider_multiple_calls() {
        // Call the function multiple times
        initialize_crypto_provider();
        initialize_crypto_provider();
        initialize_crypto_provider();

        // Should not panic or cause any issues
        // The Once ensures it only runs once internally
    }

    /// Test concurrent calls to initialize_crypto_provider
    #[test]
    fn test_initialize_crypto_provider_concurrent() {
        let handles: Vec<_> = (0..10)
            .map(|_| {
                thread::spawn(|| {
                    initialize_crypto_provider();
                })
            })
            .collect();

        // Wait for all threads to complete
        for handle in handles {
            handle.join().expect("Thread should complete successfully");
        }

        // Verify crypto provider is working after concurrent initialization
        let result = rustls::crypto::CryptoProvider::get_default()
            .unwrap()
            .secure_random
            .fill(&mut [0u8; 16]);
        assert!(result.is_ok());
    }

    /// Test crypto provider with actual TLS operations
    #[test]
    fn test_crypto_provider_tls_operations() {
        initialize_crypto_provider();

        // Test that we can create TLS-related crypto objects
        let provider = rustls::crypto::CryptoProvider::get_default().unwrap();

        // Test that cipher suites are available
        let cipher_suites = &provider.cipher_suites;
        assert!(
            !cipher_suites.is_empty(),
            "Should have available cipher suites"
        );

        // Test that we have TLS 1.3 cipher suites
        let has_tls13_suites = cipher_suites.iter().any(|suite| {
            matches!(
                suite.suite(),
                rustls::CipherSuite::TLS13_AES_256_GCM_SHA384
                    | rustls::CipherSuite::TLS13_AES_128_GCM_SHA256
                    | rustls::CipherSuite::TLS13_CHACHA20_POLY1305_SHA256
            )
        });
        assert!(
            has_tls13_suites,
            "Should have TLS 1.3 cipher suites available"
        );
    }

    /// Test error handling scenarios (if crypto provider installation fails)
    #[test]
    fn test_crypto_provider_availability() {
        initialize_crypto_provider();

        // Verify that crypto provider methods don't return errors for basic operations
        let provider = rustls::crypto::CryptoProvider::get_default().unwrap();

        // Test secure random
        let mut buffer = vec![0u8; 64];
        let random_result = provider.secure_random.fill(&mut buffer);
        assert!(
            random_result.is_ok(),
            "Secure random should work after initialization"
        );

        // Test that the buffer was filled with non-zero data
        let all_zeros = buffer.iter().all(|&b| b == 0);
        assert!(!all_zeros, "Random buffer should not be all zeros");
    }

    /// Test thread safety of the crypto provider after initialization
    #[test]
    fn test_crypto_provider_thread_safety() {
        initialize_crypto_provider();

        let success_count = Arc::new(AtomicBool::new(true));
        let handles: Vec<_> = (0..5)
            .map(|_| {
                let success_count = Arc::clone(&success_count);
                thread::spawn(move || {
                    // Test crypto operations from multiple threads
                    let provider = rustls::crypto::CryptoProvider::get_default().unwrap();
                    let mut buffer = [0u8; 16];
                    let result = provider.secure_random.fill(&mut buffer);

                    if result.is_err() {
                        success_count.store(false, Ordering::SeqCst);
                    }

                    // Small delay to increase chance of race conditions
                    thread::sleep(Duration::from_millis(1));
                })
            })
            .collect();

        // Wait for all threads
        for handle in handles {
            handle.join().expect("Thread should complete");
        }

        assert!(
            success_count.load(Ordering::SeqCst),
            "All crypto operations should succeed"
        );
    }

    /// Test that initialization is idempotent
    #[test]
    fn test_initialization_idempotent() {
        // Call initialization multiple times and verify consistent behavior
        for _ in 0..3 {
            initialize_crypto_provider();

            // Each time, verify the crypto provider works
            let provider = rustls::crypto::CryptoProvider::get_default().unwrap();
            let mut buffer = [0u8; 8];
            let result = provider.secure_random.fill(&mut buffer);
            assert!(
                result.is_ok(),
                "Crypto provider should work after each initialization"
            );
        }
    }
}
