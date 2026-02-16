//! VisionContext — shared state holding the latest screen observation.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Maximum age for a vision observation before it's considered stale.
const STALENESS_TIMEOUT: Duration = Duration::from_secs(120);

/// Holds the latest VLM description of the user's screen.
#[derive(Clone)]
pub struct VisionContext {
    inner: Arc<RwLock<VisionContextInner>>,
}

struct VisionContextInner {
    description: Option<String>,
    updated_at: Option<Instant>,
}

impl VisionContext {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(VisionContextInner {
                description: None,
                updated_at: None,
            })),
        }
    }

    /// Update with a new screen description.
    pub async fn update(&self, description: String) {
        let mut inner = self.inner.write().await;
        inner.description = Some(description);
        inner.updated_at = Some(Instant::now());
    }

    /// Get the context string for injection into chat, or None if stale/empty.
    pub async fn get_context_string(&self) -> Option<String> {
        let inner = self.inner.read().await;
        match (&inner.description, inner.updated_at) {
            (Some(desc), Some(updated_at)) => {
                if updated_at.elapsed() < STALENESS_TIMEOUT {
                    Some(desc.clone())
                } else {
                    None // Too old
                }
            }
            _ => None,
        }
    }

    /// Clear the context (e.g. when vision is disabled).
    pub async fn clear(&self) {
        let mut inner = self.inner.write().await;
        inner.description = None;
        inner.updated_at = None;
    }
}
