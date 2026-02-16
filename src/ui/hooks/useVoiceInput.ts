import { useState, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

const SAMPLE_RATE = 16000;
const VAD_THRESHOLD = 30; // Volume threshold (0-255)
const SILENCE_TIMEOUT = 1200; // ms of silence to trigger end
const MIN_SPEECH_DURATION = 500; // ms to consider valid speech

export enum VoiceState {
    Idle = "idle",
    Listening = "listening", // Mic open, waiting for speech
    Speaking = "speaking",   // User is speaking (VAD triggered)
    Processing = "processing" // Transcribing
}

export function useVoiceInput(onTranscription: (text: string) => void) {
    const [state, setState] = useState<VoiceState>(VoiceState.Idle);
    const [volume, setVolume] = useState(0); // For visualizing mic level

    const audioContext = useRef<AudioContext | null>(null);
    const mediaStream = useRef<MediaStream | null>(null);
    const processor = useRef<ScriptProcessorNode | null>(null);
    const source = useRef<MediaStreamAudioSourceNode | null>(null);

    const lastSpeechTime = useRef<number>(0);
    const speechStartTime = useRef<number>(0);
    const isSpeakingRef = useRef(false);

    // Start Microphone
    const startListening = useCallback(async () => {
        try {
            if (state !== VoiceState.Idle) return;

            const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
            mediaStream.current = stream;

            const ctx = new window.AudioContext({ sampleRate: SAMPLE_RATE });
            audioContext.current = ctx;

            const src = ctx.createMediaStreamSource(stream);
            source.current = src;

            // Use ScriptProcessor for raw data access (AudioWorklet is better but more complex to setup in Vite without specific plugin)
            // Buffer size 4096 = ~256ms at 16kHz
            const proc = ctx.createScriptProcessor(4096, 1, 1);

            proc.onaudioprocess = (e) => {
                const input = e.inputBuffer.getChannelData(0); // Float32Array (-1.0 to 1.0)

                // 1. Calculate Volume for VAD
                let sum = 0;
                for (let i = 0; i < input.length; i++) {
                    sum += input[i] * input[i];
                }
                const rms = Math.sqrt(sum / input.length);
                const db = 20 * Math.log10(rms); // usually negative
                // Map to 0-100 roughly
                const vol = Math.max(0, (db + 60) * 2);
                setVolume(vol);

                // 2. VAD Logic
                const now = Date.now();
                const isLoud = vol > VAD_THRESHOLD;

                if (isLoud) {
                    lastSpeechTime.current = now;
                    if (!isSpeakingRef.current) {
                        isSpeakingRef.current = true;
                        speechStartTime.current = now;
                        setState(VoiceState.Speaking);
                        console.log("[VAD] Speech Detected");
                    }
                }

                // 3. Silence Detection
                if (isSpeakingRef.current && !isLoud) {
                    const silenceDuration = now - lastSpeechTime.current;
                    if (silenceDuration > SILENCE_TIMEOUT) {
                        const speechDuration = lastSpeechTime.current - speechStartTime.current;

                        if (speechDuration > MIN_SPEECH_DURATION) {
                            console.log("[VAD] Silence Detected. Committing.");
                            finishStream();
                        } else {
                            console.log("[VAD] Ignored short noise.");
                            isSpeakingRef.current = false;
                            setState(VoiceState.Listening);
                            invoke("discard_audio_stream");
                        }
                    }
                }

                // 4. Send Chunk if Listening or Speaking
                // Note: we buffer even if not "speaking" yet to capture the start syllable,
                // but for simplicity in MVP we send everything while mic is open.
                // Backend Discard will clear it if needed.
                invoke("process_audio_chunk", { chunk: Array.from(input) });
            };

            src.connect(proc);
            proc.connect(ctx.destination); // Required for processing to happen in some browsers
            processor.current = proc;

            setState(VoiceState.Listening);

        } catch (err) {
            console.error("Mic Error:", err);
            setState(VoiceState.Idle);
        }
    }, [state]);

    const finishStream = useCallback(async () => {
        if (!isSpeakingRef.current) return;

        // Stop recording but keep mic open? No, let's stop/start for cleaner turns.
        // Or keep listening? User requested "Real-time", implies continuous loop.
        // For MVP, reset to Listening state after Processing.

        isSpeakingRef.current = false;
        setState(VoiceState.Processing);

        try {
            const text = await invoke<string>("complete_audio_stream");
            if (text && text.trim()) {
                onTranscription(text);
            }
        } catch (err) {
            console.error("Transcription failed:", err);
        } finally {
            // Go back to listening
            setState(VoiceState.Listening);
            // Reset timers
            lastSpeechTime.current = Date.now();
        }
    }, [onTranscription]);

    const stopListening = useCallback(() => {
        if (mediaStream.current) {
            mediaStream.current.getTracks().forEach(t => t.stop());
        }
        if (processor.current) {
            processor.current.disconnect();
        }
        if (source.current) {
            source.current.disconnect();
        }
        if (audioContext.current) {
            audioContext.current.close();
        }

        // Clean up refs
        mediaStream.current = null;
        processor.current = null;
        source.current = null;

        isSpeakingRef.current = false;
        setState(VoiceState.Idle);
        setVolume(0);

        // Cleanup backend buffer
        invoke("discard_audio_stream");
    }, []);

    return {
        state,
        volume,
        start: startListening,
        stop: stopListening
    };
}
