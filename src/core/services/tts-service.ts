import { listen, emit } from "@tauri-apps/api/event";
import { audioPlayer } from "../services";
import { VoiceInterruptService } from "./voice-interrupt-service";

interface TtsStartEvent {
    text: string;
}

interface TtsAudioEvent {
    data: number[]; // Vec<u8> comes as number array in JSON
}

interface TtsEndEvent {
    text: string;
}

interface TtsBrowserDelegateEvent {
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

    speak(text: string, voice?: string | null, speed?: number | null, pitch?: number | null): void {
        if (!this.synthesis) {
            console.warn("[TTS] SpeechSynthesis not available in this environment");
            return;
        }

        const utterance = new SpeechSynthesisUtterance(text);

        if (speed != null) utterance.rate = speed;
        if (pitch != null) utterance.pitch = pitch;

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
}

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

    async init() {
        // Tear down any previous listeners before re-registering
        this.cleanupListeners();
        const gen = ++this.generation; // unique id for THIS init() call

        console.log(`[TTS] Initializing (gen=${gen})...`);

        // Listen for Start
        const unlistenStart = await listen<TtsStartEvent>("tts:start", (_event) => {
            if (this.generation !== gen) return;
            console.log("[TTS] Started:", _event.payload.text);
            this.playbackGeneration++;
            this.isStreamActive = true;
            audioPlayer.clearQueue();
            this.browserTTS.cancel();
            this.startVoiceInterrupt();
        });
        if (this.generation !== gen) { unlistenStart(); return; }
        this.unlistenFunctions.push(unlistenStart);

        // Listen for Audio Chunks (from Rust providers)
        const unlistenAudio = await listen<TtsAudioEvent>("tts:audio", async (event) => {
            if (this.generation !== gen || !this.isStreamActive) return;
            const playbackGeneration = this.playbackGeneration;
            await audioPlayer.queueAudio(event.payload.data);
            if (this.playbackGeneration !== playbackGeneration) return;
        });
        if (this.generation !== gen) { unlistenAudio(); return; }
        this.unlistenFunctions.push(unlistenAudio);

        // Listen for Browser Delegate (frontend handles TTS)
        const unlistenBrowserDelegate = await listen<TtsBrowserDelegateEvent>(
            "tts:browser-delegate",
            (event) => {
                if (this.generation !== gen || !this.isStreamActive) return;
                const { text, voice, speed, pitch } = event.payload;
                console.log("[TTS] Browser delegate:", text.substring(0, 50));
                this.browserTTS.speak(text, voice, speed, pitch);
            }
        );
        if (this.generation !== gen) { unlistenBrowserDelegate(); return; }
        this.unlistenFunctions.push(unlistenBrowserDelegate);

        // Listen for End
        const unlistenEnd = await listen<TtsEndEvent>("tts:end", (_event) => {
            if (this.generation !== gen) return;
            console.log("[TTS] Stream ended:", _event.payload.text);
            this.isStreamActive = false;
            this.playbackGeneration++;
            audioPlayer.finishStream();
            this.stopVoiceInterrupt();
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
        this.playbackGeneration++;
        this.isStreamActive = false;
        this.cleanupListeners();
        this.browserTTS.cancel();
        this.stopVoiceInterrupt();
    }

    // ── Voice Interruption ─────────────────────────────

    private startVoiceInterrupt(): void {
        const enabled = localStorage.getItem("kokoro_voice_interrupt") === "true";
        if (!enabled) return;

        // Clean up any previous listener
        this.interruptCleanup?.();

        this.interruptCleanup = this.voiceInterrupt.onInterrupt(() => {
            console.log("[TTS] Voice interrupt triggered — stopping playback");
            this.playbackGeneration++;
            this.isStreamActive = false;
            audioPlayer.stop();
            this.browserTTS.cancel();
            this.stopVoiceInterrupt();

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
}

export const ttsService = new TtsService();
