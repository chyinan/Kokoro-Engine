//! STT Engine Interface & Core Types
//!
//! Defines the abstract contract for Speech-to-Text engines, standardized data structures
//! for audio chunks and transcription results, and semantic error handling.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

// ── Core Data Structures ────────────────────────────────

/// A standardized chunk of audio data (Monophonic Float32 PCM).
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Normalized audio samples (-1.0 to 1.0)
    pub samples: Arc<Vec<f32>>,
    /// Sampling rate (e.g. 16000)
    pub sample_rate: u32,
}

impl AudioChunk {
    /// Calculate duration in seconds
    pub fn duration_seconds(&self) -> f32 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.samples.len() as f32 / self.sample_rate as f32
        }
    }

    /// Convert to 16-bit PCM WAV bytes (mono)
    pub fn to_wav_bytes(&self) -> Vec<u8> {
        let pcm_i16: Vec<i16> = self
            .samples
            .iter()
            .map(|&sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();

        let num_channels = 1u16;
        let bits_per_sample = 16u16;
        let sample_rate = self.sample_rate;
        let byte_rate = sample_rate * num_channels as u32 * bits_per_sample as u32 / 8;
        let block_align = num_channels * bits_per_sample / 8;
        let data_size = pcm_i16.len() as u32 * 2;
        let total_size = 36 + data_size;

        let mut wav = Vec::with_capacity(44 + pcm_i16.len() * 2);

        // RIFF Chunk
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&total_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");

        // fmt Chunk
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // Chunk size
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
        wav.extend_from_slice(&num_channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());

        // data Chunk
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());

        // PCM Data
        for sample in pcm_i16 {
            wav.extend_from_slice(&sample.to_le_bytes());
        }

        wav
    }
}

/// A single segment of transcribed text with timing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionSegment {
    /// Start time in seconds relative to the beginning of the audio.
    pub start: f32,
    /// End time in seconds.
    pub end: f32,
    /// The transcribed text.
    pub text: String,
    /// Confidence score (0.0 - 1.0), if available.
    pub confidence: Option<f32>,
}

/// The full result of a transcription task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResult {
    /// The full transcribed text (concatenated segments).
    pub text: String,
    /// Detailed segments with timestamps.
    pub segments: Vec<TranscriptionSegment>,
    /// Processing duration for metrics.
    #[serde(skip)]
    pub processing_time: Duration,
}

// ── Error Handling ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SttError {
    AudioFormatInvalid(String),
    Timeout { expected: String, actual: String },
    EngineUnavailable(String),
    ModelNotLoaded,
    ChunkFailed(String),
    ProviderNotFound(String),
    ConfigError(String),
    IOError(String),
    Unknown(String),
}

impl fmt::Display for SttError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SttError::AudioFormatInvalid(msg) => write!(f, "Invalid audio format: {}", msg),
            SttError::Timeout { expected, actual } => {
                write!(f, "STT Timeout: took {} (limit: {})", actual, expected)
            }
            SttError::EngineUnavailable(msg) => write!(f, "STT Engine Unavailable: {}", msg),
            SttError::ModelNotLoaded => write!(f, "STT Model not loaded"),
            SttError::ChunkFailed(msg) => write!(f, "Chunk transcription failed: {}", msg),
            SttError::ProviderNotFound(id) => write!(f, "STT Provider not found: {}", id),
            SttError::ConfigError(msg) => write!(f, "Configuration error: {}", msg),
            SttError::IOError(msg) => write!(f, "I/O Error: {}", msg),
            SttError::Unknown(msg) => write!(f, "Unknown STT error: {}", msg),
        }
    }
}

impl std::error::Error for SttError {}

// ... (AudioChunk impl)

/// Source audio for transcription: either raw PCM chunks or encoded bytes (MP3/WAV/etc).
#[derive(Debug, Clone)]
pub enum AudioSource {
    Chunk(AudioChunk),
    Encoded { data: Vec<u8>, format: String },
}

impl AudioSource {
    pub fn duration_seconds(&self) -> f32 {
        match self {
            AudioSource::Chunk(chunk) => chunk.duration_seconds(),
            AudioSource::Encoded { data, .. } => {
                // Approximate duration for encoded audio is hard without decoding.
                // We'll use a rough heuristic: 1MB ~= 60s for mp3 (very rough).
                // Or just return 0.0 and handle it in dynamic timeout logic (if 0, use default).
                // Better: The User Blueprint asked for dynamic timeouts.
                // We'll assume encoded files are handled safely by backend or we use a generous default.
                // For now return 0.0 if unknown.
                if data.is_empty() {
                    0.0
                } else {
                    0.0
                }
            }
        }
    }
}

// ... (TranscriptionSegment, TranscriptionResult structs)

// ── Engine Trait ───────────────────────────────────────

/// Abstract interface for any STT backend (OpenAI, Local Whisper, etc.)
#[async_trait]
pub trait SttEngine: Send + Sync {
    /// Unique identifier for this engine instance
    fn id(&self) -> String;

    /// Check if the engine is ready/healthy
    async fn is_available(&self) -> bool;

    /// Transcribe audio source.
    async fn transcribe(
        &self,
        audio: &AudioSource,
        language: Option<&str>,
    ) -> Result<TranscriptionResult, SttError>;
}
