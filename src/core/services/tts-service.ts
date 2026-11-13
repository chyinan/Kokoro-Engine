import { listen, emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { cancelTts, getLatestTtsRequestId, finishChatPlayback } from "../../lib/kokoro-bridge";
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
        onError: (message: string) => void,
    ): void {
        if (!this.synthesis) {
            onError("Browser speech playback is unavailable.");
            return;
        }

        const utterance = new SpeechSynthesisUtterance(text);

        if (speed != null) utterance.rate = speed;
        if (pitch != null) utterance.pitch = pitch;
        utterance.onstart = onStart;
        utterance.onend = onEnd;
        utterance.onerror = event => onError(`Browser speech playback failed: ${event.error}`);

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
        // A default Any listener receives targeted events in every WebView.
        // Scope all playback listeners to the selected physical output window.
        const playbackTarget = { target: getCurrentWindow().label };

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
            this.failPlayback(this.activeRequestId, this.playbackGeneration, message);
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
            if (!requestId.startsWith("chat_") && latestRequestId && requestId !== latestRequestId) return;
            console.log("[TTS] Started request:", requestId);
            if (this.activeRequestId && this.activeRequestId !== requestId) {
                void finishChatPlayback(this.activeRequestId).catch(() => {});
            }
            this.playbackGeneration++;
            this.activeRequestId = requestId;
            this.isStreamActive = true;
            this.serverEnded = false;
            this.browserPending = 0;
            this.audioEventQueue = Promise.resolve();
            this.pendingAudioBytes = 0;
            audioPlayer.clearQueue();
            this.browserTTS.cancel();
        }, playbackTarget);
        if (this.generation !== gen) { unlistenStart(); return; }
        this.unlistenFunctions.push(unlistenStart);

        // Listen for Audio Chunks (from Rust providers)
        const unlistenAudio = await listen<TtsAudioEvent>("tts:audio", (event) => {
            if (this.generation !== gen || !this.isCurrentRequest(event.payload.request_id)) return;
            const playbackGeneration = this.playbackGeneration;
            const { data, format } = event.payload;
            this.pendingAudioBytes += data.length;
            if (this.pendingAudioBytes > MAX_PENDING_TTS_AUDIO_BYTES) {
                this.failPlayback(this.activeRequestId, playbackGeneration, "Buffered audio exceeded the playback limit.");
                return;
            }
            this.audioEventQueue = this.audioEventQueue
                .then(async () => {
                    if (!this.isCurrentRequest(event.payload.request_id)
                        || playbackGeneration !== this.playbackGeneration) return;
                    await audioPlayer.queueAudio(data, format);
                })
                .catch(error => {
                    this.failPlayback(event.payload.request_id ?? this.activeRequestId, playbackGeneration,
                        error instanceof Error ? error.message : String(error));
                })
                .finally(() => {
                    if (playbackGeneration === this.playbackGeneration) {
                        this.pendingAudioBytes = Math.max(0, this.pendingAudioBytes - data.length);
                    }
                });
        }, playbackTarget);
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
                try {
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
                        message => this.failPlayback(requestId, playbackGeneration, message),
                    );
                } catch (error) {
                    this.failPlayback(requestId, playbackGeneration, error instanceof Error ? error.message : String(error));
                }
            }, playbackTarget
        );
        if (this.generation !== gen) { unlistenBrowserDelegate(); return; }
        this.unlistenFunctions.push(unlistenBrowserDelegate);

        // Listen for End
        const unlistenError = await listen<TtsErrorEvent>("tts:error", (event) => {
            if (this.generation !== gen || !this.isCurrentRequest(event.payload.request_id)) return;
            console.warn("[TTS] Synthesis request failed:", event.payload.error);
        }, playbackTarget);
        if (this.generation !== gen) { unlistenError(); return; }
        this.unlistenFunctions.push(unlistenError);

        const unlistenEnd = await listen<TtsEndEvent>("tts:end", async (event) => {
            if (this.generation !== gen) return;
            if (!this.isCurrentRequest(event.payload.request_id)) return;
            const playbackGeneration = this.playbackGeneration;
            await this.audioEventQueue;
            if (playbackGeneration !== this.playbackGeneration || !this.isCurrentRequest(event.payload.request_id)) return;
            if (event.payload.status === "cancelled") {
                await this.stopPlayback(event.payload.request_id ?? this.activeRequestId);
                return;
            }
            this.serverEnded = true;
            audioPlayer.finishStream();
            this.finishIfDrained();
        }, playbackTarget);
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

    private failPlayback(requestId: string | null, generation: number, message: string): void {
        if (!requestId || generation !== this.playbackGeneration || !this.isCurrentRequest(requestId)) return;
        void this.stopPlayback(requestId, message);
        // Settings previews also use this event to report errors.
        void Promise.resolve(emit("tts:playback-error", { request_id: requestId, error: message }))
            .catch(error => console.warn("[TTS] Failed to report playback error:", error));
    }

    async stopPlayback(requestId?: string | null, error?: string): Promise<void> {
        if (this.activeRequestId) {
            const finishing = error === undefined
                ? finishChatPlayback(this.activeRequestId)
                : finishChatPlayback(this.activeRequestId, error);
            void finishing.catch(reportError => {
                if (error) console.warn("[TTS] Failed to record playback error:", reportError);
            });
        }
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
        if (!this.serverEnded || this.browserPending > 0 || audioPlayer.isPlaying
            || audioPlayer.hasPendingMediaPlayback || this.browserTTS.isSpeaking) {
            return;
        }
        if (this.activeRequestId) void finishChatPlayback(this.activeRequestId).catch(() => {});
        this.isStreamActive = false;
        this.serverEnded = false;
        this.activeRequestId = null;
        this.playbackGeneration++;
        this.stopVoiceInterrupt();
    }
}

export const ttsService = new TtsService();
