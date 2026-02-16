use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use super::interface::TtsError;

/// Concurrency-limited async queue for TTS generation.
///
/// Prevents overwhelming local models or hitting API rate limits
/// by limiting the number of concurrent synthesis requests.
pub struct TtsQueue {
    semaphore: Arc<Semaphore>,
}

impl TtsQueue {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    /// Enqueue a synthesis task. The task will execute once a semaphore permit
    /// is acquired, limiting concurrency to `max_concurrent`.
    pub fn enqueue<F, Fut>(&self, task: F) -> JoinHandle<Result<Vec<u8>, TtsError>>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<Vec<u8>, TtsError>> + Send + 'static,
    {
        let semaphore = self.semaphore.clone();
        tokio::spawn(async move {
            let _permit = semaphore
                .acquire()
                .await
                .map_err(|e| TtsError::SynthesisFailed(format!("Queue error: {}", e)))?;
            task().await
        })
    }

    /// Number of currently available permits (free slots).
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}
