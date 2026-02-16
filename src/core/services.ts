import { AudioStreamManager } from "../lib/audio-player";
import { ttsService } from "./services/tts-service";

// Singleton services
export const audioPlayer = new AudioStreamManager();
export { modService } from "./services/mod-service";
export { ttsService };
export { interactionService } from "./services/interaction-service";
