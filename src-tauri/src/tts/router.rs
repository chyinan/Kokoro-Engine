use super::interface::{ProviderCapabilities, TtsError, TtsProvider};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Smart TTS router — selects the best provider based on requested capabilities,
/// availability, and a graceful fallback chain.
///
/// Selection algorithm:
///   1. If a preferred provider is specified and available → use it
///   2. Score all available providers by capability match
///   3. Pick the highest-scoring provider
///   4. Fallback: default → browser → error
pub struct TtsRouter {
    providers: Arc<RwLock<HashMap<String, Box<dyn TtsProvider>>>>,
    default_provider: Arc<RwLock<Option<String>>>,
}

#[derive(Debug, Clone)]
pub struct RouteResult {
    pub provider_id: String,
    pub score: f32,
}

impl TtsRouter {
    pub fn new(
        providers: Arc<RwLock<HashMap<String, Box<dyn TtsProvider>>>>,
        default_provider: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            providers,
            default_provider,
        }
    }

    /// Select the best provider given optional preference and capability requirements.
    pub async fn select_provider(
        &self,
        preferred_id: Option<&str>,
        requested_caps: Option<&ProviderCapabilities>,
    ) -> Result<RouteResult, TtsError> {
        let providers = self.providers.read().await;

        if providers.is_empty() {
            return Err(TtsError::Unavailable("No TTS providers registered".into()));
        }

        // 1. Try preferred provider first
        if let Some(id) = preferred_id {
            if let Some(provider) = providers.get(id) {
                if provider.is_available().await {
                    let score = match requested_caps {
                        Some(caps) => provider.capabilities().match_score(caps),
                        None => 1.0,
                    };
                    return Ok(RouteResult {
                        provider_id: id.to_string(),
                        score,
                    });
                }
            }
        }

        // 2. Score all available providers by capability match
        let mut candidates: Vec<RouteResult> = Vec::new();
        for (id, provider) in providers.iter() {
            if !provider.is_available().await {
                continue;
            }
            let score = match requested_caps {
                Some(caps) => provider.capabilities().match_score(caps),
                None => 1.0,
            };
            candidates.push(RouteResult {
                provider_id: id.clone(),
                score,
            });
        }

        // Sort by score descending
        candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // 3. Pick the best match
        if let Some(best) = candidates.first() {
            return Ok(best.clone());
        }

        // 4. Fallback: try the default provider even if it didn't pass availability check
        let default = self.default_provider.read().await;
        if let Some(ref default_id) = *default {
            if providers.contains_key(default_id) {
                return Ok(RouteResult {
                    provider_id: default_id.clone(),
                    score: 0.0,
                });
            }
        }

        // 5. Last resort: browser
        if providers.contains_key("browser") {
            return Ok(RouteResult {
                provider_id: "browser".to_string(),
                score: 0.0,
            });
        }

        Err(TtsError::Unavailable(
            "All TTS providers are unavailable".into(),
        ))
    }
}
