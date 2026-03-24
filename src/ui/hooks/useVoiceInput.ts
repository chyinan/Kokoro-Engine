import { useState, useRef, useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauriEnvironment } from "../../utils/env";

const SNAPSHOT_INTERVAL = 2000; // 2s: ultra-fast provisional feedback
const FLUSH_WINDOW = 25000;      // 25s: force commit
const OVERLAP_SECONDS = 1.0;
const SAMPLE_RATE = 16000;
const SILENCE_MS_TO_STOP = 2500;
const SPEECH_RMS_THRESHOLD = 0.02;
type CaptureTransport = "web" | "native";

interface NativeMicVolumeEvent {
    volume: number;
    rms: number;
}

export enum VoiceState {
    Idle = "idle",
    Listening = "listening", // Active, streaming audio
    Processing = "processing", // Performing transient update
    Speaking = "speaking", // TTS active (used by UI for visualization)
    Error = "error"
}

export interface TranscriptionSegment {
    start: number;
    end: number;
    text: string;
}

interface TranscriptionResult {
    text: string;
    segments: TranscriptionSegment[];
    processing_time: any;
}

export function useVoiceInput(
    onFinalTranscription: (text: string) => void,
    onPartialTranscription?: (text: string) => void,
) {
    const [state, setState] = useState<VoiceState>(VoiceState.Idle);
    const [volume, setVolume] = useState(0);
    const [partialText, setPartialText] = useState("");

    // Store confirmed segments (history) + current active segments
    const segmentsRef = useRef<TranscriptionSegment[]>([]);

    const audioContext = useRef<AudioContext | null>(null);
    const mediaStream = useRef<MediaStream | null>(null);
    const processor = useRef<ScriptProcessorNode | null>(null);
    const sourceNode = useRef<MediaStreamAudioSourceNode | null>(null);
    const nativeVolumeUnlisten = useRef<UnlistenFn | null>(null);
    const nativeAutoStopUnlisten = useRef<UnlistenFn | null>(null);
    const captureTransport = useRef<CaptureTransport | null>(null);

    const snapshotTimer = useRef<number | null>(null);
    const lastFlushTime = useRef<number>(0);
    const isRunning = useRef(false);

    // VAD auto-stop state
    const autoStopRef = useRef(false);
    const speechDetected = useRef(false);
    const silenceSinceMsRef = useRef<number | null>(null);
    const stopRef = useRef<(() => Promise<void>) | null>(null);

    // ── Audio Processing Loop ─────────────────────────────────────────

    const handleSpeechActivity = useCallback((isSpeech: boolean) => {
        if (!autoStopRef.current) {
            return;
        }

        if (isSpeech) {
            speechDetected.current = true;
            silenceSinceMsRef.current = null;
            return;
        }

        if (!speechDetected.current) {
            return;
        }

        const now = Date.now();
        if (silenceSinceMsRef.current === null) {
            silenceSinceMsRef.current = now;
            return;
        }

        if (now - silenceSinceMsRef.current >= SILENCE_MS_TO_STOP) {
            console.log("[STT] Auto-stop: silence detected after speech");
            speechDetected.current = false;
            silenceSinceMsRef.current = null;
            setTimeout(() => stopRef.current?.(), 0);
        }
    }, []);

    const processAudioChunk = useCallback((inputData: Float32Array) => {
        // 1. Calculate Volume for UI
        let sum = 0;
        // Optimization: sample every 8th point for RMS
        for (let i = 0; i < inputData.length; i += 8) {
            sum += inputData[i] * inputData[i];
        }
        const rms = Math.sqrt(sum / (inputData.length / 8));
        const db = 20 * Math.log10(rms);
        const vol = Math.max(0, (db + 60) * 2);
        setVolume(vol);

        // 2. VAD auto-stop
        handleSpeechActivity(rms > SPEECH_RMS_THRESHOLD);

        // 3. Send to Backend
        invoke("process_audio_chunk", { chunk: Array.from(inputData) })
            .catch(err => console.warn("Audio drop:", err));
    }, [handleSpeechActivity]);

    const cleanupWebCapture = useCallback(async () => {
        if (mediaStream.current) {
            mediaStream.current.getTracks().forEach(t => t.stop());
        }
        if (sourceNode.current) sourceNode.current.disconnect();
        if (processor.current) processor.current.disconnect();
        if (audioContext.current) {
            await audioContext.current.close();
        }

        mediaStream.current = null;
        sourceNode.current = null;
        processor.current = null;
        audioContext.current = null;
    }, []);

    const cleanupNativeCapture = useCallback(async () => {
        if (nativeVolumeUnlisten.current) {
            nativeVolumeUnlisten.current();
            nativeVolumeUnlisten.current = null;
        }
        if (nativeAutoStopUnlisten.current) {
            nativeAutoStopUnlisten.current();
            nativeAutoStopUnlisten.current = null;
        }
    }, []);

    const stopCapture = useCallback(async () => {
        if (captureTransport.current === "native") {
            try {
                await invoke("stop_native_mic");
            } finally {
                await cleanupNativeCapture();
            }
            return;
        }

        await cleanupWebCapture();
    }, [cleanupNativeCapture, cleanupWebCapture]);

    const startWebCapture = useCallback(async () => {
        captureTransport.current = "web";
        const stream = await navigator.mediaDevices.getUserMedia({
            audio: {
                channelCount: 1,
                sampleRate: SAMPLE_RATE,
                echoCancellation: true,
                noiseSuppression: true
            }
        });

        mediaStream.current = stream;
        const ctx = new window.AudioContext({ sampleRate: SAMPLE_RATE });
        audioContext.current = ctx;

        const src = ctx.createMediaStreamSource(stream);
        sourceNode.current = src;

        // Use ScriptProcessor (buffer 4096 ~= 256ms)
        const proc = ctx.createScriptProcessor(4096, 1, 1);
        proc.onaudioprocess = (e) => {
            if (!isRunning.current) return;
            processAudioChunk(e.inputBuffer.getChannelData(0));
        };

        src.connect(proc);
        proc.connect(ctx.destination);
        processor.current = proc;
    }, [processAudioChunk]);

    const startNativeCapture = useCallback(async () => {
        captureTransport.current = "native";
        try {
            nativeAutoStopUnlisten.current = await listen("stt:mic-auto-stop", () => {
                if (!isRunning.current || !autoStopRef.current) return;
                console.log("[STT][native] received stt:mic-auto-stop");
                setTimeout(() => stopRef.current?.(), 0);
            });
            nativeVolumeUnlisten.current = await listen<NativeMicVolumeEvent | number>("stt:mic-volume", (event) => {
                const payload = event.payload;
                const nextVolume =
                    typeof payload === "number"
                        ? payload
                        : typeof payload?.volume === "number"
                            ? payload.volume
                            : 0;
                setVolume(nextVolume);
            });
            console.log("[STT][native] invoking start_native_mic", {
                autoStopOnSilence: autoStopRef.current,
            });
            await invoke("start_native_mic", { autoStopOnSilence: autoStopRef.current });
            console.log("[STT][native] start_native_mic resolved");
        } catch (error) {
            await cleanupNativeCapture();
            captureTransport.current = null;
            throw error;
        }
    }, [cleanupNativeCapture]);

    // ── Streaming Logic ───────────────────────────────────────────────

    const performSnapshot = useCallback(async () => {
        if (!isRunning.current) return;

        try {
            // Get partial result
            const result = await invoke<TranscriptionResult>("snapshot_audio_stream");

            // Merge logic:
            // 1. We have existing `segmentsRef.current`.
            // 2. We receive `result.segments` which has absolute timestamps (thanks to backend fix).
            // 3. We identify where the new segments start relative to our history.
            //    Since backend prunes, `result.segments` might start at t=50.0s.
            //    Our history might have segments up to t=55.0s.
            //    We should replace overlapping history with the new (likely more accurate) result.

            if (result.segments.length > 0) {
                const newSegments = result.segments;
                const firstNewStart = newSegments[0].start;

                // Keep history that ends *before* the new segments start
                // (Give a small margin for float precision / overlap adjustment if needed)
                const keepHistory = segmentsRef.current.filter(s => s.end < firstNewStart + 0.05);

                // Concatenate
                segmentsRef.current = [...keepHistory, ...newSegments];

                // Reconstruct full text
                const fullText = segmentsRef.current
                    .map(s => s.text)
                    .join("")
                    .trim();

                if (fullText) {
                    setPartialText(fullText);
                    if (onPartialTranscription) {
                        onPartialTranscription(fullText);
                    }
                }
            } else if (result.text.trim()) {
                const fallbackText = result.text.trim();
                setPartialText(fallbackText);
                if (onPartialTranscription) {
                    onPartialTranscription(fallbackText);
                }
            }

            // Check if we need to flush (commit)
            const now = Date.now();
            if (now - lastFlushTime.current > FLUSH_WINDOW) {
                // Time to prune backend!
                // We DON'T clear our local segmentsRef, because we need to accumulate text.
                // We just tell backend to discard old raw audio to save RAM.
                await invoke("prune_audio_buffer", { keepSeconds: OVERLAP_SECONDS });
                lastFlushTime.current = now;
                console.log("[STT] Pruned backend buffer (kept overlap).");
            }

        } catch (e) {
            console.error("Snapshot failed:", e);
        }
    }, [onPartialTranscription]);

    // ── Start/Stop Control ────────────────────────────────────────────

    const start = useCallback(async (opts?: { autoStopOnSilence?: boolean }) => {
        if (state !== VoiceState.Idle) return;
        const shouldAutoStop = opts?.autoStopOnSilence ?? false;
        autoStopRef.current = shouldAutoStop;

        try {
            await invoke("discard_audio_stream");

            if (isTauriEnvironment()) {
                try {
                    console.log("[STT] starting native capture", { shouldAutoStop });
                    await startNativeCapture();
                } catch (nativeError) {
                    console.warn("[STT] Native mic start failed, falling back to WebRTC:", nativeError);
                    await startWebCapture();
                }
            } else {
                await startWebCapture();
            }

            // Reset State
            isRunning.current = true;
            lastFlushTime.current = Date.now();
            speechDetected.current = false;
            silenceSinceMsRef.current = null;
            setState(VoiceState.Listening);
            setPartialText("");
            segmentsRef.current = []; // Reset history

            // Start Snapshot Loop
            snapshotTimer.current = window.setInterval(performSnapshot, SNAPSHOT_INTERVAL);

            console.log("[STT] Started listening (Streaming Mode)");

        } catch (err) {
            console.error("Failed to start mic:", err);
            await stopCapture().catch(() => undefined);
            captureTransport.current = null;
            setState(VoiceState.Error);
        }
    }, [state, performSnapshot, startNativeCapture, startWebCapture, stopCapture]);

    const stop = useCallback(async () => {
        if (!isRunning.current) return; // Guard against double-stop
        isRunning.current = false;

        // cleanup timers
        if (snapshotTimer.current) {
            clearInterval(snapshotTimer.current);
            snapshotTimer.current = null;
        }

        try {
            await stopCapture();
        } catch (e) {
            console.error("Failed to stop capture transport:", e);
        }

        // Finalize: get last bit
        try {
            const result = await invoke<TranscriptionResult>("complete_audio_stream");

            // Same merge logic one last time
            if (result.segments.length > 0) {
                const newSegments = result.segments;
                const firstNewStart = newSegments[0].start;
                const keepHistory = segmentsRef.current.filter(s => s.end < firstNewStart + 0.05);
                segmentsRef.current = [...keepHistory, ...newSegments];
            }

            const fullText =
                segmentsRef.current.map(s => s.text).join("").trim() || result.text.trim();
            if (fullText) {
                onFinalTranscription(fullText);
            }
        } catch (e) {
            console.error("Final transcription failed:", e);
        }

        captureTransport.current = null;
        speechDetected.current = false;
        silenceSinceMsRef.current = null;

        setState(VoiceState.Idle);
        setVolume(0);
        setPartialText("");
        segmentsRef.current = [];

    }, [onFinalTranscription, stopCapture]);

    // Keep stopRef in sync so VAD auto-stop can call the latest stop()
    useEffect(() => {
        stopRef.current = stop;
    }, [stop]);

    // Cleanup on unmount
    useEffect(() => {
        return () => {
            if (isRunning.current) {
                stopRef.current?.();
            } else {
                cleanupNativeCapture();
            }
        };
    }, [cleanupNativeCapture]);

    return {
        state,
        volume,
        partialText,
        start,
        stop,
        isListening: state === VoiceState.Listening
    };
}
