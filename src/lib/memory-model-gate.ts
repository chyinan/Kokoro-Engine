// pattern: Functional Core

export const MEMORY_MODEL_DIALOG_EVENT = "kokoro:open-memory-model-download-dialog";

export type MemoryModelState = "ready" | "missing" | "downloading" | "error";

export interface MemoryModelSnapshot {
    installed: boolean;
    downloading?: boolean;
    error?: string | null;
}

/** Derive a stable UI state from backend status and the local download lifecycle. */
export function deriveMemoryModelState(snapshot: MemoryModelSnapshot): MemoryModelState {
    if (snapshot.installed) {
        return "ready";
    }
    if (snapshot.error?.trim()) {
        return "error";
    }
    if (snapshot.downloading) {
        return "downloading";
    }
    return "missing";
}

/** Basic LLM turns do not depend on semantic memory being available. */
export function canStartBasicChat(_state: MemoryModelState): boolean {
    return true;
}

export function semanticRetrievalMode(state: MemoryModelState): "ready" | "unavailable" {
    return state === "ready" ? "ready" : "unavailable";
}

export function requestMemoryModelDialog(): void {
    if (typeof window === "undefined") {
        return;
    }

    window.dispatchEvent(new CustomEvent(MEMORY_MODEL_DIALOG_EVENT));
}
