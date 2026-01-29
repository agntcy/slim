// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// ============================================================================
// FFI CompletionHandle Wrapper
// ============================================================================

use parking_lot::Mutex;

use futures_timer::Delay;
use slim_session::CompletionHandle as SlimCompletionHandle;

use crate::SlimError;

/// FFI-compatible completion handle for async operations
///
/// Represents a pending operation that can be awaited to ensure completion.
/// Used for operations that need delivery confirmation or handshake acknowledgment.
///
/// # Examples
///
/// Basic usage:
/// ```ignore
/// let completion = session.publish(data, None, None)?;
/// completion.wait()?; // Wait for delivery confirmation
/// ```
#[derive(uniffi::Object)]
pub struct CompletionHandle {
    /// Receiver for the completion result (can only be consumed once)
    handle: Mutex<Option<SlimCompletionHandle>>,
}

impl std::fmt::Debug for CompletionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletionHandle")
            .field("consumed", &self.handle.lock().is_none())
            .finish()
    }
}

impl From<SlimCompletionHandle> for CompletionHandle {
    fn from(handle: SlimCompletionHandle) -> Self {
        Self {
            handle: Mutex::new(Some(handle)),
        }
    }
}

#[uniffi::export]
impl CompletionHandle {
    /// Wait for the operation to complete indefinitely (blocking version)
    ///
    /// This blocks the calling thread until the operation completes.
    /// Use this from Go or other languages when you need to ensure
    /// an operation has finished before proceeding.
    ///
    /// **Note:** This can only be called once per handle. Subsequent calls
    /// will return an error.
    ///
    /// # Returns
    /// * `Ok(())` - Operation completed successfully
    /// * `Err(SlimError)` - Operation failed or handle already consumed
    pub fn wait(self: std::sync::Arc<Self>) -> Result<(), SlimError> {
        crate::config::get_runtime().block_on(self.wait_async())
    }

    /// Wait for the operation to complete with a timeout (blocking version)
    ///
    /// This blocks the calling thread until the operation completes or the timeout expires.
    /// Use this from Go or other languages when you need to ensure
    /// an operation has finished before proceeding with a time limit.
    ///
    /// **Note:** This can only be called once per handle. Subsequent calls
    /// will return an error.
    ///
    /// # Arguments
    /// * `timeout` - Maximum time to wait for completion
    ///
    /// # Returns
    /// * `Ok(())` - Operation completed successfully
    /// * `Err(SlimError::Timeout)` - If the operation timed out
    /// * `Err(SlimError)` - Operation failed or handle already consumed
    pub fn wait_for(
        self: std::sync::Arc<Self>,
        timeout: std::time::Duration,
    ) -> Result<(), SlimError> {
        crate::config::get_runtime().block_on(self.wait_for_async(timeout))
    }

    /// Wait for the operation to complete indefinitely (async version)
    ///
    /// This is the async version that integrates with UniFFI's polling mechanism.
    /// The operation will yield control while waiting.
    ///
    /// **Note:** This can only be called once per handle. Subsequent calls
    /// will return an error.
    ///
    /// # Returns
    /// * `Ok(())` - Operation completed successfully
    /// * `Err(SlimError)` - Operation failed or handle already consumed
    pub async fn wait_async(self: std::sync::Arc<Self>) -> Result<(), SlimError> {
        self.wait_internal(None).await
    }

    /// Wait for the operation to complete with a timeout (async version)
    ///
    /// This is the async version that integrates with UniFFI's polling mechanism.
    /// The operation will yield control while waiting until completion or timeout.
    ///
    /// **Note:** This can only be called once per handle. Subsequent calls
    /// will return an error.
    ///
    /// # Arguments
    /// * `timeout` - Maximum time to wait for completion
    ///
    /// # Returns
    /// * `Ok(())` - Operation completed successfully
    /// * `Err(SlimError::Timeout)` - If the operation timed out
    /// * `Err(SlimError)` - Operation failed or handle already consumed
    pub async fn wait_for_async(
        self: std::sync::Arc<Self>,
        timeout: std::time::Duration,
    ) -> Result<(), SlimError> {
        self.wait_internal(Some(timeout)).await
    }
}

impl CompletionHandle {
    /// Internal implementation for wait operations
    async fn wait_internal(&self, timeout: Option<std::time::Duration>) -> Result<(), SlimError> {
        let receiver = self
            .handle
            .lock()
            .take()
            .ok_or_else(|| SlimError::InternalError {
                message: "CompletionHandle already consumed (wait can only be called once)"
                    .to_string(),
            })?;

        let wait_future = async { receiver.await };

        if let Some(duration) = timeout {
            // Runtime-agnostic timeout using futures-timer
            futures::pin_mut!(wait_future);
            let delay = Delay::new(duration);
            futures::pin_mut!(delay);

            match futures::future::select(wait_future, delay).await {
                futures::future::Either::Left((result, _)) => {
                    result?;
                    Ok(())
                }
                futures::future::Either::Right(_) => Err(SlimError::Timeout),
            }
        } else {
            wait_future.await?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::SlimError;
    use slim_session::SessionError as SlimSessionError;

    /// Test CompletionHandle basic functionality
    #[tokio::test]
    async fn test_completion_handle_success() {
        // Create a successful completion
        let (tx, rx) = tokio::sync::oneshot::channel();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = Arc::new(crate::CompletionHandle::from(completion));

        // Send success
        tx.send(Ok(())).unwrap();

        // Wait should succeed
        let result = ffi_handle.wait_async().await;
        assert!(result.is_ok(), "Completion should succeed");
    }

    /// Test CompletionHandle failure propagation
    #[tokio::test]
    async fn test_completion_handle_failure() {
        // Create a failed completion
        let (tx, rx) = tokio::sync::oneshot::channel();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = Arc::new(crate::CompletionHandle::from(completion));

        // Send error
        tx.send(Err(slim_session::SessionError::SlimMessageSendFailed))
            .unwrap();

        // Wait should fail with error
        let result = ffi_handle.wait_async().await;
        assert!(result.is_err_and(|e| matches!(e, SlimError::SessionError { .. })));
    }

    /// Test CompletionHandle can only be consumed once
    #[tokio::test]
    async fn test_completion_handle_single_consumption() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = Arc::new(crate::CompletionHandle::from(completion));

        tx.send(Ok(())).unwrap();

        // First wait should succeed
        let result1 = ffi_handle.clone().wait_async().await;
        assert!(result1.is_ok(), "First wait should succeed");

        // Second wait should fail (already consumed)
        let result2 = ffi_handle.clone().wait_async().await;
        assert!(result2.is_err(), "Second wait should fail");

        match result2 {
            Err(SlimError::InternalError { message }) => {
                assert!(message.contains("already consumed"));
            }
            _ => panic!("Expected InternalError about consumption"),
        }
    }

    /// Test CompletionHandle async version
    #[tokio::test]
    async fn test_completion_handle_async() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = Arc::new(crate::CompletionHandle::from(completion));

        // Send success in a separate task
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            tx.send(Ok(())).unwrap();
        });

        // Wait for completion
        let result = ffi_handle.wait_async().await;
        assert!(result.is_ok(), "Async wait should succeed");
    }

    /// Test CompletionHandle with dropped sender
    #[tokio::test]
    async fn test_completion_handle_sender_dropped() {
        let (_tx, rx) = tokio::sync::oneshot::channel();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = Arc::new(crate::CompletionHandle::from(completion));

        // Drop the sender explicitly
        drop(_tx);

        // Give the spawned task time to process
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Wait should fail with session error
        let result = ffi_handle.wait_async().await;
        assert!(
            result.is_err(),
            "Should fail when sender is dropped or already consumed"
        );

        // Either error type is valid depending on timing
        match result {
            Err(SlimError::InternalError { message }) => {
                assert!(
                    message.contains("sender dropped") || message.contains("already consumed"),
                    "Error message should mention sender dropped or already consumed, got: {}",
                    message
                );
            }
            Err(SlimError::SessionError { message }) => {
                // The background task may have consumed the receiver and got a channel closed error
                assert!(
                    message.contains("channel closed") || message.contains("receiving ack"),
                    "Expected channel closed error, got: {}",
                    message
                );
            }
            Err(e) => panic!("Unexpected error type: {:?}", e),
            Ok(_) => panic!("Expected error, got Ok"),
        }
    }

    /// Test concurrent completion handle usage
    #[tokio::test]
    async fn test_completion_handle_concurrent() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let success_count = Arc::new(AtomicU32::new(0));
        let mut handles = vec![];

        // Create multiple completion handles
        for _ in 0..10 {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
            let ffi_handle = Arc::new(crate::CompletionHandle::from(completion));

            let count = Arc::clone(&success_count);
            let handle = tokio::spawn(async move {
                // Send success
                tx.send(Ok(())).unwrap();

                // Wait for completion
                if ffi_handle.wait_async().await.is_ok() {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            });

            handles.push(handle);
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // All should succeed
        assert_eq!(success_count.load(Ordering::SeqCst), 10);
    }

    /// Test CompletionHandle with timeout
    #[tokio::test]
    async fn test_completion_handle_with_timeout() {
        // Create a completion that will never complete (sender not sent)
        // NOTE: We must hold onto `tx` so the channel stays open.
        // If we use `_tx`, it gets dropped immediately and the receiver
        // returns RecvError instead of timing out.
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), SlimSessionError>>();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = Arc::new(crate::CompletionHandle::from(completion));

        // Wait with a short timeout - should timeout
        let result = ffi_handle
            .wait_for_async(std::time::Duration::from_millis(50))
            .await;
        assert!(
            result.is_err(),
            "Should timeout when operation doesn't complete"
        );

        match result {
            Err(SlimError::Timeout) => {} // Expected
            Err(e) => panic!("Expected Timeout error, got: {:?}", e),
            Ok(_) => panic!("Expected timeout, got Ok"),
        }

        // Explicitly drop tx after the test to avoid unused variable warning
        drop(tx);
    }

    /// Test CompletionHandle with timeout that completes in time
    #[tokio::test]
    async fn test_completion_handle_timeout_success() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let completion = slim_session::CompletionHandle::from_oneshot_receiver(rx);
        let ffi_handle = Arc::new(crate::CompletionHandle::from(completion));

        // Send success immediately
        tx.send(Ok(())).unwrap();

        // Wait with a generous timeout - should succeed
        let result = ffi_handle
            .wait_for_async(std::time::Duration::from_millis(5000))
            .await;
        assert!(
            result.is_ok(),
            "Should succeed when operation completes before timeout"
        );
    }
}
