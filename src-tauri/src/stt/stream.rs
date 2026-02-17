//! Streaming Audio Handler
//!
//! Handles incoming raw PCM audio chunks from the frontend, buffers them with limits,
//! and dispatches to the STT service as standardized AudioChunks.

use crate::stt::{AudioChunk, SttService, TranscriptionResult};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

// Limit buffer to 120 seconds to prevent OOM
const SAMPLE_RATE: u32 = 16000;
const MAX_BUFFER_SECONDS: usize = 120;
const MAX_SAMPLES: usize = MAX_BUFFER_SECONDS * SAMPLE_RATE as usize;

#[derive(Debug)]
pub struct AudioStreamState {
    pub samples: Vec<f32>,
    /// How many seconds of audio have been pruned from the start.
    /// Used to adjust timestamps so they remain consistent relative to the start of the recording.
    pub time_offset_seconds: f32,
}

impl AudioStreamState {
    fn new() -> Self {
        Self {
            samples: Vec::with_capacity(SAMPLE_RATE as usize * 10),
            time_offset_seconds: 0.0,
        }
    }
}

pub struct AudioBuffer {
    pub state: Mutex<AudioStreamState>,
}

impl AudioBuffer {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(AudioStreamState::new()),
        }
    }
}

/// Append a chunk of audio data (float32 PCM, 16kHz mono).
#[tauri::command]
pub async fn process_audio_chunk(
    state: State<'_, AudioBuffer>,
    chunk: Vec<f32>,
) -> Result<(), String> {
    let mut stream = state
        .state
        .lock()
        .map_err(|_| "Failed to lock audio buffer")?;

    if stream.samples.len() + chunk.len() > MAX_SAMPLES {
        return Err(format!(
            "Audio buffer limit exceeded (max {}s). Please keep recordings short.",
            MAX_BUFFER_SECONDS
        ));
    }

    // Sanitize input: Replace NaN/Inf with 0.0 (silence) to prevent UB/Artifacts
    let sanitized_chunk = chunk.into_iter().map(|s| {
        if s.is_nan() || s.is_infinite() {
            0.0
        } else {
            s
        }
    });

    stream.samples.extend(sanitized_chunk);
    Ok(())
}

/// Finalize the stream and transcribe. Clears the buffer.
#[tauri::command]
pub async fn complete_audio_stream(
    app_handle: AppHandle,
    state: State<'_, AudioBuffer>,
) -> Result<TranscriptionResult, String> {
    let (raw_data, offset) = {
        let mut stream = state
            .state
            .lock()
            .map_err(|_| "Failed to lock audio buffer")?;
        let data = stream.samples.clone();
        let offset = stream.time_offset_seconds;
        stream.samples.clear(); // Reset for next turn
        stream.time_offset_seconds = 0.0; // Reset offset too
        (data, offset)
    };

    transcribe_helper(&app_handle, raw_data, offset).await
}

/// Transcribe the current buffer WITHOUT clearing it.
/// Useful for intermediate feedback (snapshot).
#[tauri::command]
pub async fn snapshot_audio_stream(
    app_handle: AppHandle,
    state: State<'_, AudioBuffer>,
) -> Result<TranscriptionResult, String> {
    let (raw_data, offset) = {
        let stream = state
            .state
            .lock()
            .map_err(|_| "Failed to lock audio buffer")?;
        (stream.samples.clone(), stream.time_offset_seconds)
    };

    transcribe_helper(&app_handle, raw_data, offset).await
}

/// Prune the audio buffer, keeping only the last `keep_seconds`.
/// Useful for sliding window streaming (infinite stream).
#[tauri::command]
pub async fn prune_audio_buffer(
    state: State<'_, AudioBuffer>,
    keep_seconds: f32,
) -> Result<(), String> {
    let mut stream = state
        .state
        .lock()
        .map_err(|_| "Failed to lock audio buffer")?;

    let keep_samples = (keep_seconds * SAMPLE_RATE as f32) as usize;
    if stream.samples.len() > keep_samples {
        let split_idx = stream.samples.len() - keep_samples;

        // Calculate how many seconds we are pruning to update the offset
        let pruned_seconds = split_idx as f32 / SAMPLE_RATE as f32;
        stream.time_offset_seconds += pruned_seconds;

        stream.samples.drain(0..split_idx);
    }

    Ok(())
}

/// Discard current buffer without transcribing.
#[tauri::command]
pub async fn discard_audio_stream(state: State<'_, AudioBuffer>) -> Result<(), String> {
    let mut stream = state
        .state
        .lock()
        .map_err(|_| "Failed to lock audio buffer")?;
    stream.samples.clear();
    stream.time_offset_seconds = 0.0;
    Ok(())
}

/// Helper function to encapsulate transcription logic
async fn transcribe_helper(
    app_handle: &AppHandle,
    raw_data: Vec<f32>,
    time_offset: f32,
) -> Result<TranscriptionResult, String> {
    if raw_data.is_empty() {
        return Ok(TranscriptionResult {
            text: String::new(),
            segments: Vec::new(),
            processing_time: std::time::Duration::ZERO,
        });
    }

    // Construct standardized AudioChunk
    let chunk = AudioChunk {
        samples: std::sync::Arc::new(raw_data),
        sample_rate: SAMPLE_RATE,
    };

    // Get STT Service
    let stt_service = app_handle.state::<SttService>();

    // Transcribe
    let mut result = stt_service
        .transcribe(&crate::stt::AudioSource::Chunk(chunk), None)
        .await
        .map_err(|e| e.to_string())?;

    // Apply time offset correction to segments
    if time_offset > 0.0 {
        for segment in &mut result.segments {
            segment.start += time_offset;
            segment.end += time_offset;
        }
    }

    Ok(result)
}
