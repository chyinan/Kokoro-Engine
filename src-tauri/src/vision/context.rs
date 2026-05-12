//! VisionContext — shared state for captured frames and completed observations.

use crate::vision::config::NormalizedRect;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Maximum age for a vision observation before it's considered stale.
pub const STALENESS_TIMEOUT: Duration = Duration::from_secs(120);
const FOCUS_RESUME_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct VisionFrame {
    pub id: String,
    pub captured_at: DateTime<Utc>,
    pub jpeg_bytes: Vec<u8>,
    pub display_id: Option<String>,
    pub region: Option<NormalizedRect>,
    pub image_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisionObservationSource {
    Auto,
    ManualTool,
}

impl VisionObservationSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::ManualTool => "manual_tool",
        }
    }
}

#[derive(Debug, Clone)]
pub struct VisionObservation {
    pub id: String,
    pub frame_id: Option<String>,
    pub captured_at: DateTime<Utc>,
    pub analyzed_at: DateTime<Utc>,
    pub summary: String,
    pub source: VisionObservationSource,
}

#[derive(Debug, Clone)]
pub enum AnalysisDispatch {
    Dispatch(VisionFrame),
    NotDispatched,
}

#[derive(Clone)]
pub struct VisionContext {
    inner: Arc<RwLock<VisionContextInner>>,
}

#[derive(Debug)]
pub struct VisionContextInner {
    pub latest_frame: Option<VisionFrame>,
    pub latest_auto_observation: Option<VisionObservation>,
    pub latest_manual_observation: Option<VisionObservation>,
    pub analysis_in_flight: bool,
    pub pending_frame: Option<VisionFrame>,
    pub last_error: Option<String>,
    pub text_input_focused: bool,
    pub resume_auto_capture_after: Option<Instant>,
}

impl Default for VisionContext {
    fn default() -> Self {
        Self::new()
    }
}

impl VisionContext {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(VisionContextInner {
                latest_frame: None,
                latest_auto_observation: None,
                latest_manual_observation: None,
                analysis_in_flight: false,
                pending_frame: None,
                last_error: None,
                text_input_focused: false,
                resume_auto_capture_after: None,
            })),
        }
    }

    pub async fn latest_completed_observation(
        &self,
        now: DateTime<Utc>,
    ) -> Option<VisionObservation> {
        let inner = self.inner.read().await;
        newest_non_stale(
            inner.latest_auto_observation.as_ref(),
            inner.latest_manual_observation.as_ref(),
            now,
            STALENESS_TIMEOUT,
        )
    }

    pub async fn submit_auto_frame(&self, frame: VisionFrame) -> AnalysisDispatch {
        let mut inner = self.inner.write().await;
        inner.latest_frame = Some(frame.clone());
        if inner.analysis_in_flight {
            inner.pending_frame = Some(frame);
            AnalysisDispatch::NotDispatched
        } else {
            inner.analysis_in_flight = true;
            AnalysisDispatch::Dispatch(frame)
        }
    }

    pub async fn finish_auto_analysis(
        &self,
        frame: &VisionFrame,
        result: Result<String, String>,
    ) -> Option<VisionFrame> {
        let mut inner = self.inner.write().await;
        match result {
            Ok(summary) => {
                inner.last_error = None;
                inner.latest_auto_observation = Some(VisionObservation {
                    id: uuid::Uuid::new_v4().to_string(),
                    frame_id: Some(frame.id.clone()),
                    captured_at: frame.captured_at,
                    analyzed_at: Utc::now(),
                    summary,
                    source: VisionObservationSource::Auto,
                });
            }
            Err(error) => {
                inner.last_error = Some(error);
            }
        }

        if let Some(next) = inner.pending_frame.take() {
            Some(next)
        } else {
            inner.analysis_in_flight = false;
            None
        }
    }

    pub async fn record_manual_observation(&self, observation: VisionObservation) {
        let mut inner = self.inner.write().await;
        inner.latest_manual_observation = Some(observation);
    }

    pub async fn set_text_input_focus_state(&self, focused: bool) {
        let mut inner = self.inner.write().await;
        inner.text_input_focused = focused;
        inner.resume_auto_capture_after = if focused {
            None
        } else {
            Some(Instant::now() + FOCUS_RESUME_DELAY)
        };
    }

    pub async fn should_pause_auto_capture(&self) -> bool {
        let inner = self.inner.read().await;
        inner.text_input_focused
            || inner
                .resume_auto_capture_after
                .is_some_and(|resume_at| Instant::now() < resume_at)
    }

    pub async fn clear_auto_state_on_disable(&self) {
        let mut inner = self.inner.write().await;
        inner.latest_frame = None;
        inner.latest_auto_observation = None;
        inner.analysis_in_flight = false;
        inner.pending_frame = None;
        inner.last_error = None;
        inner.resume_auto_capture_after = None;
    }

    pub async fn set_last_error(&self, error: String) {
        self.inner.write().await.last_error = Some(error);
    }

    /// Legacy compatibility for callers that only need a context string.
    pub async fn get_context_string(&self) -> Option<String> {
        self.latest_completed_observation(Utc::now())
            .await
            .map(|observation| observation.summary)
    }

    /// Clear all context (used when the watcher is explicitly stopped).
    pub async fn clear(&self) {
        let mut inner = self.inner.write().await;
        inner.latest_frame = None;
        inner.latest_auto_observation = None;
        inner.latest_manual_observation = None;
        inner.analysis_in_flight = false;
        inner.pending_frame = None;
        inner.last_error = None;
        inner.resume_auto_capture_after = None;
    }
}

fn newest_non_stale(
    auto: Option<&VisionObservation>,
    manual: Option<&VisionObservation>,
    now: DateTime<Utc>,
    staleness_timeout: Duration,
) -> Option<VisionObservation> {
    [auto, manual]
        .into_iter()
        .flatten()
        .filter(|observation| {
            now.signed_duration_since(observation.analyzed_at)
                .to_std()
                .map(|age| age <= staleness_timeout)
                .unwrap_or(true)
        })
        .max_by_key(|observation| observation.analyzed_at)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(
        source: VisionObservationSource,
        analyzed_at: DateTime<Utc>,
    ) -> VisionObservation {
        VisionObservation {
            id: uuid::Uuid::new_v4().to_string(),
            frame_id: None,
            captured_at: analyzed_at,
            analyzed_at,
            summary: format!("{:?}", source),
            source,
        }
    }

    fn frame(hash: &str) -> VisionFrame {
        VisionFrame {
            id: uuid::Uuid::new_v4().to_string(),
            captured_at: Utc::now(),
            jpeg_bytes: vec![1, 2, 3],
            display_id: None,
            region: None,
            image_hash: hash.to_string(),
        }
    }

    #[tokio::test]
    async fn latest_completed_observation_selects_newest_non_stale_auto_or_manual() {
        let ctx = VisionContext::new();
        let now = Utc::now();
        ctx.record_manual_observation(observation(
            VisionObservationSource::ManualTool,
            now - chrono::Duration::seconds(30),
        ))
        .await;
        let auto_frame = frame("auto");
        ctx.finish_auto_analysis(&auto_frame, Ok("auto".to_string()))
            .await;

        let selected = ctx.latest_completed_observation(now).await.unwrap();
        assert_eq!(selected.source, VisionObservationSource::Auto);
    }

    #[tokio::test]
    async fn manual_observation_expiry_falls_back_to_auto() {
        let ctx = VisionContext::new();
        let now = Utc::now();
        ctx.record_manual_observation(observation(
            VisionObservationSource::ManualTool,
            now - chrono::Duration::seconds(180),
        ))
        .await;
        ctx.record_manual_observation(observation(
            VisionObservationSource::Auto,
            now - chrono::Duration::seconds(30),
        ))
        .await;
        let auto = observation(
            VisionObservationSource::Auto,
            now - chrono::Duration::seconds(30),
        );
        ctx.inner.write().await.latest_auto_observation = Some(auto);

        let selected = ctx.latest_completed_observation(now).await.unwrap();
        assert_eq!(selected.source, VisionObservationSource::Auto);
    }

    #[tokio::test]
    async fn pending_frame_keeps_newest_only_and_one_in_flight() {
        let ctx = VisionContext::new();
        let first = frame("first");
        let second = frame("second");
        let third = frame("third");

        assert!(matches!(
            ctx.submit_auto_frame(first.clone()).await,
            AnalysisDispatch::Dispatch(_)
        ));
        assert!(matches!(
            ctx.submit_auto_frame(second).await,
            AnalysisDispatch::NotDispatched
        ));
        assert!(matches!(
            ctx.submit_auto_frame(third.clone()).await,
            AnalysisDispatch::NotDispatched
        ));

        let next = ctx
            .finish_auto_analysis(&first, Ok("done".to_string()))
            .await
            .unwrap();
        assert_eq!(next.image_hash, "third");
        assert!(ctx.inner.read().await.analysis_in_flight);
        assert!(ctx
            .finish_auto_analysis(&next, Ok("done2".to_string()))
            .await
            .is_none());
        assert!(!ctx.inner.read().await.analysis_in_flight);
    }

    #[tokio::test]
    async fn clear_auto_state_on_disable_drops_completed_auto_observation() {
        let ctx = VisionContext::new();
        let now = Utc::now();
        ctx.inner.write().await.latest_auto_observation = Some(observation(
            VisionObservationSource::Auto,
            now - chrono::Duration::seconds(30),
        ));

        ctx.clear_auto_state_on_disable().await;

        assert!(ctx.latest_completed_observation(now).await.is_none());
    }

    #[tokio::test]
    async fn text_input_focus_pauses_auto_capture_until_resume_delay_expires() {
        let ctx = VisionContext::new();

        ctx.set_text_input_focus_state(true).await;
        assert!(ctx.should_pause_auto_capture().await);

        ctx.set_text_input_focus_state(false).await;
        assert!(ctx.should_pause_auto_capture().await);

        ctx.inner.write().await.resume_auto_capture_after =
            Some(Instant::now() - Duration::from_millis(1));
        assert!(!ctx.should_pause_auto_capture().await);
    }
}
