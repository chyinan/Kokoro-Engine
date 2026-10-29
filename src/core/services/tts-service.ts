import { listen, emit } from "@tauri-apps/api/event";
import { cancelTts, getLatestTtsRequestId } from "../../lib/kokoro-bridge";
import type { TtsAudioFormat } from "../../core/types/mod";
import { audioPlayer } from "../services";
import { VoiceInterruptService } from "./voice-interrupt-service";

interface TtsStartEvent {
    request_id?: string;
    text: string;
}

interface TtsAudioEvent {
    request_id?: string;
    sentence_id?: number;
    sequence?: number;
    response_id?: string | null;
    format?: TtsAudioFormat | null;
    data: number[]; // Vec<u8> comes as number array in JSON
}

interface TtsEndEvent {
    request_id?: string;
    text: string;
    status?: "completed" | "cancelled" | "error";
}

interface TtsErrorEvent {
    request_id?: string;
    error: string;
}

interface TtsBrowserDelegateEvent {
    request_id?: string;
    sentence_id?: number;
    text: string;
    voice: string | null;
    speed: number | null;
    pitch: number | null;
}

/**
 * Browser-native SpeechSynthesis wrapper.
 * Used as fallback when the Rust backend signals BrowserDelegate.
 */
class BrowserSpeechSynthesis {
    private synthesis: SpeechSynthesis | null;

    constructor() {
        this.synthesis = typeof window !== "undefined" ? window.speechSynthesis : null;
    }

    speak(
        text: string,
        voice: string | null | undefined,
        speed: number | null | undefined,
        pitch: number | null | undefined,
        onStart: () => void,
        onEnd: () => void,
    ): void {
        if (!this.synthesis) {
            console.warn("[TTS] SpeechSynthesis not available in this environment");
            return;
        }

        const utterance = new SpeechSynthesisUtterance(text);

        if (speed != null) utterance.rate = speed;
        if (pitch != null) utterance.pitch = pitch;
        utterance.onstart = onStart;
        utterance.onend = onEnd;
        utterance.onerror = onEnd;

        // Try to find the requested voice
        if (voice) {
            const voices = this.synthesis.getVoices();
            const match = voices.find(
                v => v.name === voice || v.voiceURI === voice || v.lang.startsWith(voice)
            );
            if (match) utterance.voice = match;
        }

        this.synthesis.speak(utterance);
    }

    cancel(): void {
        this.synthesis?.cancel();
    }

    get isAvailable(): boolean {
        return this.synthesis !== null;
    }

    get isSpeaking(): boolean {
        return this.synthesis?.speaking ?? false;
    }
}

const MAX_PENDING_TTS_AUDIO_BYTES = 1_000_000;

export class TtsService {
    private unlistenFunctions: (() => void)[] = [];
    private browserTTS = new BrowserSpeechSynthesis();
    private voiceInterrupt = new VoiceInterruptService();
    private interruptCleanup: (() => void) | null = null;
    /** Generation counter — incremented on each init/cleanup to invalidate stale async listeners */
    private generation = 0;
    /** Playback generation — invalidates audio that arrives after a stream is interrupted/ended. */
    private playbackGeneration = 0;
    private isStreamActive = false;
    private activeRequestId: string | null = null;
    private serverEnded = false;
    private browserPending = 0;
    private audioEventQueue: Promise<void> = Promise.resolve();
    private pendingAudioBytes = 0;
    private removePlayStateListener: (() => void) | null = null;
    private removePlaybackErrorListener: (() => void) | null = null;
    private removeStopEventListener: (() => void) | null = null;
    private removeReplacementEventListener: (() => void) | null = null;

    async init(): Promise<void> {
        // Tear down any previous listeners before re-registering
        this.cleanupListeners();
        this.removePlayStateListener?.();
        this.removePlaybackErrorListener?.();
        this.removeStopEventListener?.();
        this.removeReplacementEventListener?.();
        const gen = ++this.generation; // unique id for THIS init() call

        console.log(`[TTS] Initializing (gen=${gen})...`);

        this.removePlayStateListener = audioPlayer.onPlayStateChange((playing) => {
            if (this.generation !== gen || !this.isStreamActive) return;
            if (playing) {
                this.startVoiceInterrupt();
            } else {
                this.stopVoiceInterrupt();
                this.finishIfDrained();
            }
        });
        this.removePlaybackErrorListener = audioPlayer.onPlaybackError(message => {
            const requestId = this.activeRequestId;
            if (!requestId) return;
            void Promise.resolve(emit("tts:playback-error", { request_id: requestId, error: message }))
                .catch(error => console.warn("[TTS] Failed to report playback error:", error));
            void this.stopPlayback(requestId);
        });
        if (typeof window !== "undefined" && typeof window.addEventListener === "function") {
            const stopPlayback = () => { void this.stopPlayback(); };
            window.addEventListener("kokoro-tts-stop", stopPlayback);
            this.removeStopEventListener = () => window.removeEventListener("kokoro-tts-stop", stopPlayback);

            const replacePlayback = (event: Event) => {
                const detail = (event as CustomEvent<{ replaced_request_id?: string | null }>).detail;
                const replacedId = detail?.replaced_request_id;
                if (replacedId && this.activeRequestId === replacedId) {
                    void this.stopPlayback(replacedId);
                }
            };
            window.addEventListener("kokoro-tts-replacing", replacePlayback);
            this.removeReplacementEventListener = () => window.removeEventListener("kokoro-tts-replacing", replacePlayback);
        }

        // Listen for Start
        const unlistenStart = await listen<TtsStartEvent>("tts:start", (event) => {
            if (this.generation !== gen) return;
            const requestId = event.payload.request_id ?? getLatestTtsRequestId() ?? "legacy-tts-request";
            const latestRequestId = getLatestTtsRequestId();
            if (latestRequestId && requestId !== latestRequestId) return;
            console.log("[TTS] Started request:", requestId);
            this.playbackGeneration++;
            this.activeRequestId = requestId;
            this.isStreamActive = true;
            this.serverEnded = false;
            this.browserPending = 0;
            this.audioEventQueue = Promise.resolve();
            this.pendingAudioBytes = 0;
            audioPlayer.clearQueue();
            this.browserTTS.cancel();
        });
        if (this.generation !== gen) { unlistenStart(); return; }
        this.unlistenFunctions.push(unlistenStart);

        // Listen for Audio Chunks (from Rust providers)
        const unlistenAudio = await listen<TtsAudioEvent>("tts:audio", (event) => {
            if (this.generation !== gen || !this.isCurrentRequest(event.payload.request_id)) return;
            const playbackGeneration = this.playbackGeneration;
            const { data, format } = event.payload;
            this.pendingAudioBytes += data.length;
            if (this.pendingAudioBytes > MAX_PENDING_TTS_AUDIO_BYTES) {
                console.warn("[TTS] Buffered audio exceeded the playback limit; cancelling the request");
                void this.stopPlayback(event.payload.request_id ?? this.activeRequestId);
                return;
            }
            this.audioEventQueue = this.audioEventQueue
                .then(async () => {
                    if (!this.isCurrentRequest(event.payload.request_id)
                        || playbackGeneration !== this.playbackGeneration) return;
                    await audioPlayer.queueAudio(data, format);
                })
                .catch(error => {
                    if (!this.isCurrentRequest(event.payload.request_id)) return;
                    console.warn("[TTS] Audio playback failed:", error);
                    void Promise.resolve(emit("tts:playback-error", {
                        request_id: event.payload.request_id ?? this.activeRequestId ?? "",
                        error: error instanceof Error ? error.message : String(error),
                    })).catch(reportError => console.warn("[TTS] Failed to report playback error:", reportError));
                    void this.stopPlayback(event.payload.request_id ?? this.activeRequestId);
                })
                .finally(() => {
                    if (playbackGeneration === this.playbackGeneration) {
                        this.pendingAudioBytes = Math.max(0, this.pendingAudioBytes - data.length);
                    }
                });
        });
        if (this.generation !== gen) { unlistenAudio(); return; }
        this.unlistenFunctions.push(unlistenAudio);

        // Listen for Browser Delegate (frontend handles TTS)
        const unlistenBrowserDelegate = await listen<TtsBrowserDelegateEvent>(
            "tts:browser-delegate",
            (event) => {
                if (this.generation !== gen || !this.isCurrentRequest(event.payload.request_id)) return;
                const { text, voice, speed, pitch } = event.payload;
                const requestId = this.activeRequestId;
                const playbackGeneration = this.playbackGeneration;
                this.browserPending++;
                this.browserTTS.speak(
                    text,
                    voice,
                    speed,
                    pitch,
                    () => {
                        if (requestId && this.isCurrentRequest(requestId)
                            && playbackGeneration === this.playbackGeneration) {
                            this.startVoiceInterrupt();
                        }
                    },
                    () => this.onBrowserUtteranceEnd(requestId, playbackGeneration),
                );
            }
        );
        if (this.generation !== gen) { unlistenBrowserDelegate(); return; }
        this.unlistenFunctions.push(unlistenBrowserDelegate);

        // Listen for End
        const unlistenError = await listen<TtsErrorEvent>("tts:error", (event) => {
            if (this.generation !== gen || !this.isCurrentRequest(event.payload.request_id)) return;
            console.warn("[TTS] Synthesis request failed:", event.payload.error);
        });
        if (this.generation !== gen) { unlistenError(); return; }
        this.unlistenFunctions.push(unlistenError);

        const unlistenEnd = await listen<TtsEndEvent>("tts:end", async (event) => {
            if (this.generation !== gen) return;
            if (!this.isCurrentRequest(event.payload.request_id)) return;
            await this.audioEventQueue;
            if (!this.isCurrentRequest(event.payload.request_id)) return;
            if (event.payload.status === "cancelled") {
                await this.stopPlayback(event.payload.request_id ?? this.activeRequestId);
                return;
            }
            this.serverEnded = true;
            audioPlayer.finishStream();
            this.finishIfDrained();
        });
        if (this.generation !== gen) { unlistenEnd(); return; }
        this.unlistenFunctions.push(unlistenEnd);
    }

    private cleanupListeners() {
        this.unlistenFunctions.forEach(fn => fn());
        this.unlistenFunctions = [];
    }

    cleanup() {
        this.generation++; // invalidate any in-flight init()
        this.cleanupListeners();
        this.removePlayStateListener?.();
        this.removePlayStateListener = null;
        this.removePlaybackErrorListener?.();
        this.removePlaybackErrorListener = null;
        this.removeStopEventListener?.();
        this.removeStopEventListener = null;
        this.removeReplacementEventListener?.();
        this.removeReplacementEventListener = null;
        void this.stopPlayback();
    }

    async stopPlayback(requestId?: string | null): Promise<void> {
        const requestIdsToCancel = requestId
            ? [requestId]
            : [...new Set([this.activeRequestId, getLatestTtsRequestId()].filter((id): id is string => Boolean(id)))];
        this.playbackGeneration++;
        this.activeRequestId = null;
        this.isStreamActive = false;
        this.serverEnded = false;
        this.browserPending = 0;
        this.pendingAudioBytes = 0;
        this.audioEventQueue = Promise.resolve();
        audioPlayer.stop();
        this.browserTTS.cancel();
        this.stopVoiceInterrupt();
        for (const requestToCancel of requestIdsToCancel) {
            try {
                await cancelTts(requestToCancel);
            } catch (error) {
                // Cancellation is best-effort and should never be surfaced as a network alert.
                console.warn("[TTS] Backend cancellation request failed:", error);
            }
        }
    }

    // ── Voice Interruption ─────────────────────────────

    private startVoiceInterrupt(): void {
        const enabled = localStorage.getItem("kokoro_voice_interrupt") === "true";
        if (!enabled) return;

        if (this.interruptCleanup) return;
        const requestId = this.activeRequestId;
        this.interruptCleanup = this.voiceInterrupt.onInterrupt(() => {
            console.log("[TTS] Voice interrupt triggered — stopping playback");
            void this.stopPlayback(requestId);

            // Bridge to STT: emit event so ChatPanel can start listening
            const sttEnabled = localStorage.getItem("kokoro_stt_enabled") === "true";
            if (sttEnabled) {
                emit("voice-interrupt-stt", {});
            }
        });

        this.voiceInterrupt.start().catch(err => {
            console.warn("[TTS] Failed to start voice interrupt:", err);
        });
    }

    private stopVoiceInterrupt(): void {
        this.voiceInterrupt.stop();
        this.interruptCleanup?.();
        this.interruptCleanup = null;
    }

    private isCurrentRequest(requestId?: string | null): boolean {
        return this.isStreamActive
            && (requestId == null || this.activeRequestId === requestId);
    }

    private onBrowserUtteranceEnd(requestId: string | null, playbackGeneration: number): void {
        if (!requestId || !this.isCurrentRequest(requestId)
            || playbackGeneration !== this.playbackGeneration) return;
        this.browserPending = Math.max(0, this.browserPending - 1);
        if (this.browserPending === 0) this.stopVoiceInterrupt();
        this.finishIfDrained();
    }

    private finishIfDrained(): void {
        if (!this.serverEnded || this.browserPending > 0 || audioPlayer.isPlaying || this.browserTTS.isSpeaking) {
            return;
        }
        this.isStreamActive = false;
        this.serverEnded = false;
        this.activeRequestId = null;
        this.playbackGeneration++;
        this.stopVoiceInterrupt();
    }
}

export const ttsService = new TtsService();
