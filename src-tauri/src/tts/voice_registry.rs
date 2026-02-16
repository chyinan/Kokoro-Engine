use super::interface::VoiceProfile;
use std::collections::HashMap;

/// Central registry of all available voices across all providers.
pub struct VoiceRegistry {
    voices: HashMap<String, VoiceProfile>,
}

impl VoiceRegistry {
    pub fn new() -> Self {
        Self {
            voices: HashMap::new(),
        }
    }

    /// Register a voice profile. Overwrites if voice_id already exists.
    pub fn register(&mut self, profile: VoiceProfile) {
        self.voices.insert(profile.voice_id.clone(), profile);
    }

    /// Bulk-register voices from a provider.
    pub fn register_all(&mut self, profiles: Vec<VoiceProfile>) {
        for profile in profiles {
            self.register(profile);
        }
    }

    /// Get a voice by ID.
    pub fn get(&self, voice_id: &str) -> Option<&VoiceProfile> {
        self.voices.get(voice_id)
    }

    /// List all registered voices.
    pub fn list(&self) -> Vec<&VoiceProfile> {
        self.voices.values().collect()
    }

    /// Find voices by language code (e.g., "en", "zh").
    pub fn find_by_language(&self, lang: &str) -> Vec<&VoiceProfile> {
        self.voices
            .values()
            .filter(|v| v.language.starts_with(lang))
            .collect()
    }

    /// Find voices by engine type.
    pub fn find_by_engine(&self, engine: &super::interface::TtsEngine) -> Vec<&VoiceProfile> {
        self.voices
            .values()
            .filter(|v| &v.engine == engine)
            .collect()
    }

    /// Find voices belonging to a specific provider.
    pub fn find_by_provider(&self, provider_id: &str) -> Vec<&VoiceProfile> {
        self.voices
            .values()
            .filter(|v| v.provider_id == provider_id)
            .collect()
    }

    /// Remove all voices from a specific provider.
    pub fn remove_provider_voices(&mut self, provider_id: &str) {
        self.voices.retain(|_, v| v.provider_id != provider_id);
    }
}
