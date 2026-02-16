//! Streaming Audio Handler
//!
//! Handles incoming raw PCM audio chunks from the frontend, buffers them,
//! and converts them to WAV format for STT transcription.

use crate::stt::SttService;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

pub struct AudioBuffer {
    pub data: Mutex<Vec<f32>>,
}

impl AudioBuffer {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Vec::with_capacity(16000 * 10)), // Pre-allocate ~10s
        }
    }
}

/// Append a chunk of audio data (float32 PCM, 16kHz mono).
#[tauri::command]
pub async fn process_audio_chunk(
    state: State<'_, AudioBuffer>,
    chunk: Vec<f32>,
) -> Result<(), String> {
    let mut buffer = state
        .data
        .lock()
        .map_err(|_| "Failed to lock audio buffer")?;
    buffer.extend_from_slice(&chunk);
    Ok(())
}

/// Finalize the stream, convert into WAV, and send to STT provider.
/// Returns the transcribed text.
#[tauri::command]
pub async fn complete_audio_stream(
    app_handle: AppHandle,
    state: State<'_, AudioBuffer>,
) -> Result<String, String> {
    let raw_data = {
        let mut buffer = state
            .data
            .lock()
            .map_err(|_| "Failed to lock audio buffer")?;
        let data = buffer.clone();
        buffer.clear(); // Reset for next turn
        data
    };

    if raw_data.is_empty() {
        return Ok(String::new());
    }

    // Convert f32 (-1.0..1.0) to i16 PCM
    let pcm_i16: Vec<i16> = raw_data
        .iter()
        .map(|&sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect();

    // Create WAV container (16kHz, 1 channel, 16-bit)
    let wav_bytes = create_wav_header(&pcm_i16, 16000);

    // Get STT Service
    let stt_service = app_handle.state::<SttService>();

    // Transcribe
    // "wav" is the format hint, "en" or auto-detect is fine.
    // Ideally we pass language from frontend, but for now let's rely on auto-detect or config defaults in SttService.
    let text = stt_service
        .transcribe(&wav_bytes, "wav", None)
        .await
        .map_err(|e| e.to_string())?;

    Ok(text)
}

/// Discard current buffer without transcribing.
#[tauri::command]
pub async fn discard_audio_stream(state: State<'_, AudioBuffer>) -> Result<(), String> {
    let mut buffer = state
        .data
        .lock()
        .map_err(|_| "Failed to lock audio buffer")?;
    buffer.clear();
    Ok(())
}

fn create_wav_header(pcm_data: &[i16], sample_rate: u32) -> Vec<u8> {
    let num_channels = 1u16;
    let bits_per_sample = 16u16;
    let byte_rate = sample_rate * num_channels as u32 * bits_per_sample as u32 / 8;
    let block_align = num_channels * bits_per_sample / 8;
    let data_size = pcm_data.len() as u32 * 2;
    let total_size = 36 + data_size;

    let mut wav = Vec::with_capacity(44 + pcm_data.len() * 2);

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
    for sample in pcm_data {
        wav.extend_from_slice(&sample.to_le_bytes());
    }

    wav
}
