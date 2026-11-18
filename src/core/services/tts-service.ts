import { events } from "@/lib/generated/ipc";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { cancelTts, getLatestTtsRequestId, finishChatPlayback } from "../../lib/kokoro-bridge";
import { chatApplication, type ChatApplication } from "../../lib/chat/application";
import { chatEventMatchesView } from "../../lib/chat/projection";
import type { ChatPlaybackScope } from "../../lib/kokoro-bridge";
import { audioPlayer } from "../services";
import { VoiceInterruptService } from "./voice-interrupt-service";

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
    constructor(private readonly chats: Pick<ChatApplication, "getSnapshot" | "subscribe" | "refresh"> = chatApplication) {}

    private activeChatScope: ChatPlaybackScope | null = null;
    private startSequence = 0;
    private pendingStart: { requestId: string; ready: Promise<void>; audioBytes: number } | null = null;
    private removeSelectionListener: (() => void) | null = null;
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
        const playbackWindow = getCurrentWindow();

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
        const unlistenStart = await events.ttsStart(playbackWindow).listen((event) => {
            if (this.generation !== gen) return;
            const requestId = event.payload.request_id ?? getLatestTtsRequestId() ?? "legacy-tts-request";
            const latestRequestId = getLatestTtsRequestId();
            if (!requestId.startsWith("chat_") && latestRequestId && requestId !== latestRequestId) return;
            if (this.pendingStart && this.pendingStart.requestId !== requestId) this.rejectStart(this.pendingStart.requestId);
            const sequence = ++this.startSequence;
            const scope = event.payload.chat_scope;
            if (!scope) {
                this.pendingStart = null;
                this.acceptStart(requestId, null);
                return;
            }
            // Validate before replacing any current output. A late A start must
            // not clear a valid B/manual stream while its scope is being checked.
            const ready = this.chats.refresh().then(() => {
                if (this.generation !== gen || sequence !== this.startSequence) return;
                if (this.scopeMatchesView(scope)) this.acceptStart(requestId, scope);
                else this.rejectStart(requestId);
            }, () => {
                if (sequence === this.startSequence) this.rejectStart(requestId);
            }).finally(() => {
                if (this.pendingStart?.ready === ready) this.pendingStart = null;
            });
            this.pendingStart = { requestId, ready, audioBytes: 0 };
            return ready;
        });
        if (this.generation !== gen) { unlistenStart(); return; }
        this.unlistenFunctions.push(unlistenStart);

        // Listen for Audio Chunks (from Rust providers)
        const unlistenAudio = await events.ttsAudio(playbackWindow).listen(async (event) => {
            if (this.pendingStart && this.pendingStart.requestId === event.payload.request_id) {
                const pending = this.pendingStart;
                pending.audioBytes += event.payload.data.length;
                if (pending.audioBytes > MAX_PENDING_TTS_AUDIO_BYTES) {
                    ++this.startSequence;
                    this.pendingStart = null;
                    this.rejectStart(pending.requestId);
                    return;
                }
                await pending.ready;
            }
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
        });
        if (this.generation !== gen) { unlistenAudio(); return; }
        this.unlistenFunctions.push(unlistenAudio);

        // Listen for Browser Delegate (frontend handles TTS)
        const unlistenBrowserDelegate = await events.ttsBrowserDelegate(playbackWindow).listen(async (event) => {
                if (this.pendingStart && this.pendingStart.requestId === event.payload.request_id) await this.pendingStart.ready;
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
            });
        if (this.generation !== gen) { unlistenBrowserDelegate(); return; }
        this.unlistenFunctions.push(unlistenBrowserDelegate);

        // Listen for End
        const unlistenError = await events.ttsError(playbackWindow).listen(async (event) => {
            if (this.pendingStart && this.pendingStart.requestId === event.payload.request_id) await this.pendingStart.ready;
            if (this.generation !== gen || !this.isCurrentRequest(event.payload.request_id)) return;
            console.warn("[TTS] Synthesis request failed:", event.payload.error);
        });
        if (this.generation !== gen) { unlistenError(); return; }
        this.unlistenFunctions.push(unlistenError);

        const unlistenEnd = await events.ttsEnd(playbackWindow).listen(async (event) => {
            if (this.pendingStart && this.pendingStart.requestId === event.payload.request_id) await this.pendingStart.ready;
            if (this.generation !== gen) return;
            if (!this.isCurrentRequest(event.payload.request_id)) return;
            if (event.payload.status === "cancelled") {
                // Invalidate pending audio before it can report an error for this cancelled request.
                await this.stopPlayback(event.payload.request_id ?? this.activeRequestId);
                return;
            }
            const playbackGeneration = this.playbackGeneration;
            await this.audioEventQueue;
            if (playbackGeneration !== this.playbackGeneration || !this.isCurrentRequest(event.payload.request_id)) return;
            this.serverEnded = true;
            audioPlayer.finishStream();
            this.finishIfDrained();
        });
        if (this.generation !== gen) { unlistenEnd(); return; }
        this.unlistenFunctions.push(unlistenEnd);
    }

    private acceptStart(requestId: string, scope: ChatPlaybackScope | null): void {
        if (this.activeRequestId && this.activeRequestId !== requestId) {
            void finishChatPlayback(this.activeRequestId).catch(() => {});
        }
        this.playbackGeneration++;
        this.activeRequestId = requestId;
        this.activeChatScope = scope;
        this.isStreamActive = true;
        this.serverEnded = false;
        this.browserPending = 0;
        this.audioEventQueue = Promise.resolve();
        this.pendingAudioBytes = 0;
        audioPlayer.clearQueue();
        this.browserTTS.cancel();
        if (scope) {
            this.removeSelectionListener ??= this.chats.subscribe(() => {
                if (this.activeChatScope && !this.scopeMatchesView()) void this.stopPlayback(this.activeRequestId);
            });
        }
    }

    private rejectStart(requestId: string): void {
        void cancelTts(requestId).catch(() => {});
        void finishChatPlayback(requestId).catch(() => {});
    }

    private cleanupListeners() {
        ++this.startSequence;
        if (this.pendingStart) this.rejectStart(this.pendingStart.requestId);
        this.pendingStart = null;
        this.removeSelectionListener?.();
        this.removeSelectionListener = null;
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
        void Promise.resolve(events.ttsPlaybackError.emit({ request_id: requestId, error: message }))
            .catch(error => console.warn("[TTS] Failed to report playback error:", error));
    }

    async stopPlayback(requestId?: string | null, error?: string): Promise<void> {
        if (this.pendingStart && (!requestId || this.pendingStart.requestId === requestId)) {
            ++this.startSequence;
            this.rejectStart(this.pendingStart.requestId);
            this.pendingStart = null;
        }
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
        this.activeChatScope = null;
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
                events.voiceInterruptStt.emit({});
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

    private scopeMatchesView(scope = this.activeChatScope): boolean {
        if (!scope) return true; // Explicit manual speech has no chat-view lifetime.
        const view = this.chats.getSnapshot();
        return view.generation === scope.selection_generation && chatEventMatchesView(view, scope);
    }

    private isCurrentRequest(requestId?: string | null): boolean {
        return this.isStreamActive
            && this.scopeMatchesView()
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
        this.activeChatScope = null;
        this.playbackGeneration++;
        this.stopVoiceInterrupt();
    }
}

export const ttsService = new TtsService();
