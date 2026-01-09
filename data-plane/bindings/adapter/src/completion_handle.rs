// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// ============================================================================
// FFI CompletionHandle Wrapper
// ============================================================================

use parking_lot::Mutex;

use slim_session::CompletionHandle as SlimCompletionHandle;

use crate::{SlimError, runtime};

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
        runtime::get_runtime().block_on(self.wait_async())
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
        runtime::get_runtime().block_on(self.wait_for_async(timeout))
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

        let wait_future = async { receiver.await.map_err(SlimError::from) };

        if let Some(duration) = timeout {
            tokio::time::timeout(duration, wait_future)
                .await
                .map_err(|_| SlimError::Timeout)??;
            Ok(())
        } else {
            wait_future.await?;
            Ok(())
        }
    }
}
