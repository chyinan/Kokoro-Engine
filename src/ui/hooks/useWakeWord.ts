import { useRef, useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

const SAMPLE_RATE = 16000;
const FRAME_SIZE = 1024; // ~64ms per frame

// VAD thresholds
const SPEECH_RMS_THRESHOLD = 0.01;   // above → speech
const SILENCE_RMS_THRESHOLD = 0.005; // below → silence
const SPEECH_ONSET_FRAMES = 3;       // frames above threshold to start recording (~192ms)
const SILENCE_END_FRAMES = 24;       // frames below threshold to end recording (~1.5s)
const MIN_SPEECH_FRAMES = 5;         // discard clips shorter than ~320ms
const MAX_SPEECH_FRAMES = 125;       // max ~8s clip

export interface WakeWordOptions {
    wakeWord: string;           // e.g. "你好心音"
    enabled: boolean;
    onWakeWordDetected: () => void;
}

export function useWakeWord({ wakeWord, enabled, onWakeWordDetected }: WakeWordOptions) {
    const audioContextRef = useRef<AudioContext | null>(null);
    const mediaStreamRef = useRef<MediaStream | null>(null);
    const processorRef = useRef<ScriptProcessorNode | null>(null);
    const sourceRef = useRef<MediaStreamAudioSourceNode | null>(null);
    const isRunningRef = useRef(false);

    // VAD state
    const speechFramesRef = useRef(0);
    const silenceFramesRef = useRef(0);
    const isRecordingRef = useRef(false);
    const clipBufferRef = useRef<number[]>([]);

    // Refs to avoid stale closures in processFrame
    const wakeWordRef = useRef(wakeWord);
    const onWakeWordDetectedRef = useRef(onWakeWordDetected);
    useEffect(() => { wakeWordRef.current = wakeWord; }, [wakeWord]);
    useEffect(() => { onWakeWordDetectedRef.current = onWakeWordDetected; }, [onWakeWordDetected]);

    const checkWakeWord = useCallback(async (samples: number[]) => {
        try {
            const text: string = await invoke("transcribe_wake_word_audio", { samples });
            if (!text) return;
            const normalized = text.toLowerCase().replace(/\s+/g, "");
            const keyword = wakeWordRef.current.toLowerCase().replace(/\s+/g, "");
            if (!keyword) return; // guard against whitespace-only wake word
            if (normalized.includes(keyword)) {
                onWakeWordDetectedRef.current();
            }
        } catch (e) {
            console.warn("[WakeWord] Transcription failed:", e);
        }
    }, []);

    const processFrame = useCallback((frame: Float32Array) => {
        // RMS energy
        let sum = 0;
        for (let i = 0; i < frame.length; i++) {
            sum += frame[i] * frame[i];
        }
        const rms = Math.sqrt(sum / frame.length);

        if (!isRecordingRef.current) {
            // Waiting for speech onset
            if (rms > SPEECH_RMS_THRESHOLD) {
                speechFramesRef.current++;
                if (speechFramesRef.current >= SPEECH_ONSET_FRAMES) {
                    isRecordingRef.current = true;
                    silenceFramesRef.current = 0;
                    clipBufferRef.current = Array.from(frame);
                }
            } else {
                speechFramesRef.current = 0;
            }
        } else {
            // Recording clip — use concat to avoid spread of 1024 args
            const arr = Array.from(frame);
            clipBufferRef.current = clipBufferRef.current.concat(arr);

            if (rms < SILENCE_RMS_THRESHOLD) {
                silenceFramesRef.current++;
            } else {
                silenceFramesRef.current = 0;
            }

            const totalFrames = clipBufferRef.current.length / FRAME_SIZE;

            const shouldEnd =
                silenceFramesRef.current >= SILENCE_END_FRAMES ||
                totalFrames >= MAX_SPEECH_FRAMES;

            if (shouldEnd) {
                const clip = clipBufferRef.current.slice();
                isRecordingRef.current = false;
                speechFramesRef.current = 0;
                silenceFramesRef.current = 0;
                clipBufferRef.current = [];

                if (totalFrames >= MIN_SPEECH_FRAMES) {
                    checkWakeWord(clip);
                }
            }
        }
    }, [checkWakeWord]);

    const start = useCallback(async () => {
        if (isRunningRef.current) return;
        try {
            const stream = await navigator.mediaDevices.getUserMedia({
                audio: {
                    channelCount: 1,
                    sampleRate: SAMPLE_RATE,
                    echoCancellation: true,
                    noiseSuppression: true,
                }
            });
            mediaStreamRef.current = stream;
            const ctx = new AudioContext({ sampleRate: SAMPLE_RATE });
            audioContextRef.current = ctx;
            const src = ctx.createMediaStreamSource(stream);
            sourceRef.current = src;
            const proc = ctx.createScriptProcessor(FRAME_SIZE, 1, 1);
            proc.onaudioprocess = (e) => {
                if (!isRunningRef.current) return;
                processFrame(e.inputBuffer.getChannelData(0));
            };
            src.connect(proc);
            proc.connect(ctx.destination);
            processorRef.current = proc;
            isRunningRef.current = true;
            console.log("[WakeWord] Listening for:", wakeWordRef.current);
        } catch (e) {
            console.error("[WakeWord] Failed to start mic:", e);
        }
    }, [processFrame]);

    const stop = useCallback(() => {
        isRunningRef.current = false;
        isRecordingRef.current = false;
        speechFramesRef.current = 0;
        silenceFramesRef.current = 0;
        clipBufferRef.current = [];
        mediaStreamRef.current?.getTracks().forEach(t => t.stop());
        sourceRef.current?.disconnect();
        processorRef.current?.disconnect();
        audioContextRef.current?.close();
        mediaStreamRef.current = null;
        sourceRef.current = null;
        processorRef.current = null;
        audioContextRef.current = null;
    }, []);

    useEffect(() => {
        if (enabled && wakeWord.trim()) {
            start();
        } else {
            stop();
        }
        return () => stop();
    }, [enabled, wakeWord]); // eslint-disable-line react-hooks/exhaustive-deps

    return { stop };
}
