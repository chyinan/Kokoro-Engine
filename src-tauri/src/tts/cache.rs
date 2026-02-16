use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// LRU-based TTS audio cache with TTL expiration.
///
/// Cache key is derived from (text, voice_id, provider_id, speed, pitch).
/// Thread safety is handled at the TtsService level via Arc<RwLock<TtsCache>>.
pub struct TtsCache {
    entries: HashMap<CacheKey, CacheEntry>,
    max_entries: usize,
    ttl: Duration,
    /// Access order for LRU eviction (most recently used at the end)
    access_order: Vec<CacheKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    text: String,
    voice_id: String,
    provider_id: String,
    /// Speed × 100 as integer for hashing
    speed_centis: i32,
    /// Pitch × 100 as integer for hashing
    pitch_centis: i32,
}

struct CacheEntry {
    audio: Vec<u8>,
    created_at: Instant,
}

impl CacheKey {
    pub fn new(
        text: &str,
        voice_id: &str,
        provider_id: &str,
        speed: Option<f32>,
        pitch: Option<f32>,
    ) -> Self {
        Self {
            text: text.to_string(),
            voice_id: voice_id.to_string(),
            provider_id: provider_id.to_string(),
            speed_centis: (speed.unwrap_or(1.0) * 100.0) as i32,
            pitch_centis: (pitch.unwrap_or(1.0) * 100.0) as i32,
        }
    }
}

impl TtsCache {
    pub fn new(max_entries: usize, ttl_secs: u64) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            ttl: Duration::from_secs(ttl_secs),
            access_order: Vec::new(),
        }
    }

    /// Try to retrieve cached audio. Returns None if not found or expired.
    pub fn get(&mut self, key: &CacheKey) -> Option<Vec<u8>> {
        if let Some(entry) = self.entries.get(key) {
            // Check TTL
            if entry.created_at.elapsed() > self.ttl {
                // Expired — remove it
                self.entries.remove(key);
                self.access_order.retain(|k| k != key);
                return None;
            }
            // Update access order (move to end)
            self.access_order.retain(|k| k != key);
            self.access_order.push(key.clone());
            return Some(entry.audio.clone());
        }
        None
    }

    /// Store audio in the cache. Evicts LRU entries if over capacity.
    pub fn put(&mut self, key: CacheKey, audio: Vec<u8>) {
        // Evict if at capacity
        while self.entries.len() >= self.max_entries && !self.access_order.is_empty() {
            let oldest = self.access_order.remove(0);
            self.entries.remove(&oldest);
        }

        self.access_order.retain(|k| k != &key);
        self.access_order.push(key.clone());
        self.entries.insert(
            key,
            CacheEntry {
                audio,
                created_at: Instant::now(),
            },
        );
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.access_order.clear();
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
