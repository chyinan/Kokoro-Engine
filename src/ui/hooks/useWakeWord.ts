// pattern: Imperative Shell

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
    const captureGenerationRef = useRef(0);
    const startOwnerRef = useRef(0);
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

    // A stale native start must always send a stop command.  The normal stop
    // path can legitimately return early after another cleanup cleared the
    // transport ref, while the Rust worker may still be finishing its start.
    const forceStopNativeCapture = useCallback(async (owner = startOwnerRef.current) => {
        try {
            await invoke("stop_native_wake_word", { owner });
        } catch (error) {
            console.warn("[WakeWord] Failed to force-stop native wake word:", error);
        } finally {
            // An older stop must not clear listeners or transport state that a
            // newer owner has already installed.
            if (startOwnerRef.current === owner) {
                await cleanupNativeCapture();
                transportRef.current = null;
            }
        }
    }, [cleanupNativeCapture]);

    const stop = useCallback(async () => {
        if (!isRunningRef.current && !transportRef.current) {
            return;
        }
        const owner = startOwnerRef.current;

        // Invalidate any getUserMedia/AudioContext setup that is still awaiting.
        captureGenerationRef.current += 1;
        isRunningRef.current = false;
        isRecordingRef.current = false;
        speechFramesRef.current = 0;
        silenceFramesRef.current = 0;
        clipBufferRef.current = [];

        if (transportRef.current === "native") {
            await forceStopNativeCapture(owner);
        } else {
            await cleanupWebCapture();
        }

        if (startOwnerRef.current === owner) {
            transportRef.current = null;
        }
    }, [cleanupWebCapture, forceStopNativeCapture]);

    const triggerWakeWordDetected = useCallback(async (text?: string, owner?: number) => {
        if (owner !== undefined && owner !== startOwnerRef.current) {
            return;
        }
        if (detectionInFlightRef.current) {
            return;
        }

        detectionInFlightRef.current = true;
        try {
            await stopRef.current?.();
            if (owner !== undefined && owner !== startOwnerRef.current) {
                return;
            }
            onWakeWordDetectedRef.current(text);
        } finally {
            detectionInFlightRef.current = false;
            if (
                (owner === undefined || owner === startOwnerRef.current)
                && modeRef.current === "speech"
                && enabledRef.current
            ) {
                setTimeout(() => {
                    void startRef.current?.();
                }, 0);
            }
        }
    }, []);

    const checkWakeWord = useCallback(async (
        samples: number[],
        captureGeneration: number,
        captureOwner: number,
    ) => {
        try {
            if (modeRef.current === "speech") {
                const text: string = await invoke("transcribe_wake_word_audio", { samples });
                if (
                    captureGeneration !== captureGenerationRef.current
                    || captureOwner !== startOwnerRef.current
                ) return;
                const trimmed = text.trim();
                if (trimmed) {
                    await triggerWakeWordDetected(trimmed, captureOwner);
                }
                return;
            }
            const text: string = await invoke("transcribe_wake_word_audio", { samples });
            if (
                captureGeneration !== captureGenerationRef.current
                || captureOwner !== startOwnerRef.current
            ) return;
            if (!text) return;
            const normalized = text.toLowerCase().replace(/\s+/g, "");
            const keyword = wakeWordRef.current.toLowerCase().replace(/\s+/g, "");
            if (!keyword) return;
            if (normalized.includes(keyword)) {
                await triggerWakeWordDetected(undefined, captureOwner);
            }
        } catch (error) {
            console.warn("[WakeWord] Transcription failed:", error);
        }
    }, [triggerWakeWordDetected]);

    const processFrame = useCallback((
        frame: Float32Array,
        captureGeneration: number,
        captureOwner: number,
    ) => {
        if (
            captureGeneration !== captureGenerationRef.current
            || captureOwner !== startOwnerRef.current
        ) return;
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
            void checkWakeWord(clip, captureGeneration, captureOwner);
        }
    }, [checkWakeWord]);

    const startWebCapture = useCallback(async () => {
        const captureGeneration = captureGenerationRef.current;
        const captureOwner = startOwnerRef.current;
        transportRef.current = "web";
        const stream = await navigator.mediaDevices.getUserMedia({
            audio: {
                channelCount: 1,
                sampleRate: SAMPLE_RATE,
                echoCancellation: true,
                noiseSuppression: true,
            },
        });
        if (
            captureGeneration !== captureGenerationRef.current
            || transportRef.current !== "web"
        ) {
            stream.getTracks().forEach((track) => track.stop());
            return;
        }
        mediaStreamRef.current = stream;

        try {
            const context = new AudioContext({ sampleRate: SAMPLE_RATE });
            audioContextRef.current = context;
            if (
                captureGeneration !== captureGenerationRef.current
                || transportRef.current !== "web"
            ) {
                stream.getTracks().forEach((track) => track.stop());
                await context.close();
                return;
            }
            const source = context.createMediaStreamSource(stream);
            sourceRef.current = source;
            const processor = context.createScriptProcessor(FRAME_SIZE, 1, 1);
            processor.onaudioprocess = (event) => {
                if (!isRunningRef.current) return;
                processFrame(
                    event.inputBuffer.getChannelData(0),
                    captureGeneration,
                    captureOwner,
                );
            };

            source.connect(processor);
            processor.connect(context.destination);
            processorRef.current = processor;
        } catch (error) {
            stream.getTracks().forEach((track) => track.stop());
            await cleanupWebCapture();
            throw error;
        }
    }, [cleanupWebCapture, processFrame]);

    const startNativeCapture = useCallback(async (owner: number) => {
        const captureGeneration = captureGenerationRef.current;
        transportRef.current = "native";
        let detectedUnlisten: UnlistenFn | null = null;
        let errorUnlisten: UnlistenFn | null = null;
        try {
            detectedUnlisten = await listen<string>("stt:wake-word-detected", (event) => {
                if (startOwnerRef.current !== owner) return;
                void triggerWakeWordDetected(event.payload, owner);
            });
            if (startOwnerRef.current !== owner) {
                detectedUnlisten();
                return;
            }
            nativeDetectedUnlisten.current?.();
            nativeDetectedUnlisten.current = detectedUnlisten;
            errorUnlisten = await listen<string>("stt:wake-word-error", (event) => {
                if (startOwnerRef.current !== owner) return;
                console.warn("[WakeWord] Native wake word error:", event.payload);
            });
            if (startOwnerRef.current !== owner) {
                errorUnlisten();
                // The current ref may already belong to a newer owner. Only
                // remove the listener installed by this start attempt.
                if (nativeDetectedUnlisten.current === detectedUnlisten) {
                    nativeDetectedUnlisten.current();
                    nativeDetectedUnlisten.current = null;
                }
                return;
            }
            nativeErrorUnlisten.current?.();
            nativeErrorUnlisten.current = errorUnlisten;
            await invoke("start_native_wake_word", {
                wakeWord: wakeWordRef.current,
                triggerOnSpeech: modeRef.current === "speech",
                owner,
            });
            // The stop request may have won the race while the native start
            // command was in flight.  Compensate after the command returns so
            // a late Rust worker cannot keep recording after this start dies.
            if (
                captureGeneration !== captureGenerationRef.current
                || startOwnerRef.current !== owner
                || transportRef.current !== "native"
            ) {
                // Only stop this owner's worker. A newer start may already
                // own the native stream and must not be interrupted here.
                if (startOwnerRef.current === owner) {
                    await forceStopNativeCapture(owner);
                }
            }
        } catch (error) {
            if (startOwnerRef.current === owner) {
                await cleanupNativeCapture();
                transportRef.current = null;
            } else {
                detectedUnlisten?.();
                errorUnlisten?.();
            }
            throw error;
        }
    }, [cleanupNativeCapture, forceStopNativeCapture, triggerWakeWordDetected]);

    const start = useCallback(async () => {
        if (isRunningRef.current) return;

        const captureGeneration = captureGenerationRef.current;
        const owner = startOwnerRef.current + 1;
        startOwnerRef.current = owner;
        try {
            if (isTauriEnvironment()) {
                try {
                    await startNativeCapture(owner);
                } catch (error) {
                    if (startOwnerRef.current !== owner) return;
                    console.warn("[WakeWord] Native wake word start failed, falling back to WebRTC:", error);
                    await startWebCapture();
                }
            } else {
                await startWebCapture();
            }

            if (
                captureGeneration !== captureGenerationRef.current
                || startOwnerRef.current !== owner
                || !enabledRef.current
            ) {
                // The native start request can complete after a concurrent
                // stop. Send a compensating stop so the Rust worker cannot
                // remain active after this stale start is discarded.
                if (startOwnerRef.current === owner && isTauriEnvironment()) {
                    await forceStopNativeCapture(owner);
                } else if (startOwnerRef.current === owner) {
                    await stop();
                }
                return;
            }
            isRunningRef.current = true;
        } catch (error) {
            if (startOwnerRef.current === owner) {
                transportRef.current = null;
            }
            console.error("[WakeWord] Failed to start microphone:", error);
        }
    }, [forceStopNativeCapture, startNativeCapture, startWebCapture, stop]);

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
