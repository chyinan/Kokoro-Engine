import { useRef, useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isTauriEnvironment } from "../../utils/env";

const SAMPLE_RATE = 16000;
const FRAME_SIZE = 1024;

const SPEECH_RMS_THRESHOLD = 0.01;
const SILENCE_RMS_THRESHOLD = 0.005;
const SPEECH_ONSET_FRAMES = 3;
const SILENCE_END_FRAMES = 24;
const MIN_SPEECH_FRAMES = 5;
const MAX_SPEECH_FRAMES = 125;

type WakeWordTransport = "web" | "native";
type DetectionMode = "wake_word" | "speech";

export interface WakeWordOptions {
    wakeWord?: string;
    enabled: boolean;
    mode?: DetectionMode;
    onWakeWordDetected: (text?: string) => void;
}

export function useWakeWord({ wakeWord = "", enabled, mode = "wake_word", onWakeWordDetected }: WakeWordOptions) {
    const audioContextRef = useRef<AudioContext | null>(null);
    const mediaStreamRef = useRef<MediaStream | null>(null);
    const processorRef = useRef<ScriptProcessorNode | null>(null);
    const sourceRef = useRef<MediaStreamAudioSourceNode | null>(null);
    const nativeDetectedUnlisten = useRef<UnlistenFn | null>(null);
    const nativeErrorUnlisten = useRef<UnlistenFn | null>(null);
    const transportRef = useRef<WakeWordTransport | null>(null);
    const isRunningRef = useRef(false);
    const detectionInFlightRef = useRef(false);
    const enabledRef = useRef(enabled);
    const startRef = useRef<(() => Promise<void>) | null>(null);
    const stopRef = useRef<(() => Promise<void>) | null>(null);

    const speechFramesRef = useRef(0);
    const silenceFramesRef = useRef(0);
    const isRecordingRef = useRef(false);
    const clipBufferRef = useRef<number[]>([]);

    const wakeWordRef = useRef(wakeWord);
    const modeRef = useRef<DetectionMode>(mode);
    const onWakeWordDetectedRef = useRef(onWakeWordDetected);
    useEffect(() => {
        wakeWordRef.current = wakeWord;
    }, [wakeWord]);
    useEffect(() => {
        modeRef.current = mode;
    }, [mode]);
    useEffect(() => {
        enabledRef.current = enabled;
    }, [enabled]);
    useEffect(() => {
        onWakeWordDetectedRef.current = onWakeWordDetected;
    }, [onWakeWordDetected]);

    const cleanupWebCapture = useCallback(async () => {
        mediaStreamRef.current?.getTracks().forEach((track) => track.stop());
        sourceRef.current?.disconnect();
        processorRef.current?.disconnect();
        if (audioContextRef.current) {
            await audioContextRef.current.close();
        }

        mediaStreamRef.current = null;
        sourceRef.current = null;
        processorRef.current = null;
        audioContextRef.current = null;
    }, []);

    const cleanupNativeCapture = useCallback(async () => {
        if (nativeDetectedUnlisten.current) {
            nativeDetectedUnlisten.current();
            nativeDetectedUnlisten.current = null;
        }
        if (nativeErrorUnlisten.current) {
            nativeErrorUnlisten.current();
            nativeErrorUnlisten.current = null;
        }
    }, []);

    const stop = useCallback(async () => {
        if (!isRunningRef.current && !transportRef.current) {
            return;
        }

        isRunningRef.current = false;
        isRecordingRef.current = false;
        speechFramesRef.current = 0;
        silenceFramesRef.current = 0;
        clipBufferRef.current = [];

        if (transportRef.current === "native") {
            try {
                await invoke("stop_native_wake_word");
            } catch (error) {
                console.warn("[WakeWord] Failed to stop native wake word:", error);
            } finally {
                await cleanupNativeCapture();
            }
        } else {
            await cleanupWebCapture();
        }

        transportRef.current = null;
    }, [cleanupNativeCapture, cleanupWebCapture]);

    const triggerWakeWordDetected = useCallback(async (text?: string) => {
        if (detectionInFlightRef.current) {
            return;
        }

        detectionInFlightRef.current = true;
        try {
            await stopRef.current?.();
            onWakeWordDetectedRef.current(text);
        } finally {
            detectionInFlightRef.current = false;
            if (modeRef.current === "speech" && enabledRef.current) {
                setTimeout(() => {
                    void startRef.current?.();
                }, 0);
            }
        }
    }, []);

    const checkWakeWord = useCallback(async (samples: number[]) => {
        try {
            if (modeRef.current === "speech") {
                const text: string = await invoke("transcribe_wake_word_audio", { samples });
                const trimmed = text.trim();
                if (trimmed) {
                    await triggerWakeWordDetected(trimmed);
                }
                return;
            }
            const text: string = await invoke("transcribe_wake_word_audio", { samples });
            if (!text) return;
            const normalized = text.toLowerCase().replace(/\s+/g, "");
            const keyword = wakeWordRef.current.toLowerCase().replace(/\s+/g, "");
            if (!keyword) return;
            if (normalized.includes(keyword)) {
                await triggerWakeWordDetected();
            }
        } catch (error) {
            console.warn("[WakeWord] Transcription failed:", error);
        }
    }, [triggerWakeWordDetected]);

    const processFrame = useCallback((frame: Float32Array) => {
        let sum = 0;
        for (let i = 0; i < frame.length; i++) {
            sum += frame[i] * frame[i];
        }
        const rms = Math.sqrt(sum / frame.length);

        if (!isRecordingRef.current) {
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
            return;
        }

        clipBufferRef.current = clipBufferRef.current.concat(Array.from(frame));

        if (rms < SILENCE_RMS_THRESHOLD) {
            silenceFramesRef.current++;
        } else {
            silenceFramesRef.current = 0;
        }

        const totalFrames = clipBufferRef.current.length / FRAME_SIZE;
        const shouldEnd =
            silenceFramesRef.current >= SILENCE_END_FRAMES ||
            totalFrames >= MAX_SPEECH_FRAMES;

        if (!shouldEnd) {
            return;
        }

        const clip = clipBufferRef.current.slice();
        isRecordingRef.current = false;
        speechFramesRef.current = 0;
        silenceFramesRef.current = 0;
        clipBufferRef.current = [];

        if (totalFrames >= MIN_SPEECH_FRAMES) {
            void checkWakeWord(clip);
        }
    }, [checkWakeWord]);

    const startWebCapture = useCallback(async () => {
        transportRef.current = "web";
        const stream = await navigator.mediaDevices.getUserMedia({
            audio: {
                channelCount: 1,
                sampleRate: SAMPLE_RATE,
                echoCancellation: true,
                noiseSuppression: true,
            },
        });
        mediaStreamRef.current = stream;

        const context = new AudioContext({ sampleRate: SAMPLE_RATE });
        audioContextRef.current = context;
        const source = context.createMediaStreamSource(stream);
        sourceRef.current = source;
        const processor = context.createScriptProcessor(FRAME_SIZE, 1, 1);
        processor.onaudioprocess = (event) => {
            if (!isRunningRef.current) return;
            processFrame(event.inputBuffer.getChannelData(0));
        };

        source.connect(processor);
        processor.connect(context.destination);
        processorRef.current = processor;
    }, [processFrame]);

    const startNativeCapture = useCallback(async () => {
        transportRef.current = "native";
        try {
            nativeDetectedUnlisten.current = await listen<string>("stt:wake-word-detected", (event) => {
                void triggerWakeWordDetected(event.payload);
            });
            nativeErrorUnlisten.current = await listen<string>("stt:wake-word-error", (event) => {
                console.warn("[WakeWord] Native wake word error:", event.payload);
            });
            await invoke("start_native_wake_word", {
                wakeWord: wakeWordRef.current,
                triggerOnSpeech: modeRef.current === "speech",
            });
        } catch (error) {
            await cleanupNativeCapture();
            transportRef.current = null;
            throw error;
        }
    }, [cleanupNativeCapture, triggerWakeWordDetected]);

    const start = useCallback(async () => {
        if (isRunningRef.current) return;

        try {
            if (isTauriEnvironment()) {
                try {
                    await startNativeCapture();
                } catch (error) {
                    console.warn("[WakeWord] Native wake word start failed, falling back to WebRTC:", error);
                    await startWebCapture();
                }
            } else {
                await startWebCapture();
            }

            isRunningRef.current = true;
        } catch (error) {
            transportRef.current = null;
            console.error("[WakeWord] Failed to start microphone:", error);
        }
    }, [startNativeCapture, startWebCapture]);

    useEffect(() => {
        startRef.current = start;
    }, [start]);

    useEffect(() => {
        stopRef.current = stop;
    }, [stop]);

    useEffect(() => {
        if (enabled && (mode === "speech" || wakeWord.trim())) {
            void start();
        } else {
            void stop();
        }

        return () => {
            void stop();
        };
    }, [enabled, start, stop, wakeWord]);

    return { stop };
}
