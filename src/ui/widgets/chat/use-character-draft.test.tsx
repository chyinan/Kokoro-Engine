// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
    loadSavedCharacterDraft,
    saveCharacterDraft,
} from "./chat-draft-layout";
import { useCharacterChatDraft } from "./use-character-draft";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("useCharacterChatDraft", () => {
    let container: HTMLDivElement | null = null;
    let root: Root | null = null;
    let mockStore: Record<string, string> = {};
    let mockStorage: Storage;

    beforeEach(() => {
        vi.useFakeTimers();
        mockStore = {};
        mockStorage = {
            getItem: (k: string) => mockStore[k] ?? null,
            setItem: (k: string, v: string) => {
                mockStore[k] = v;
            },
            removeItem: (k: string) => {
                delete mockStore[k];
            },
            clear: () => {
                mockStore = {};
            },
            key: () => null,
            length: 0,
        };

        container = document.createElement("div");
        document.body.appendChild(container);
        root = createRoot(container);
    });

    afterEach(() => {
        if (root && container) {
            act(() => {
                root?.unmount();
            });
            container.remove();
        }
        root = null;
        container = null;
        vi.useRealTimers();
    });

    function TestHarness({
        characterId,
        onHook,
    }: {
        characterId: string;
        onHook: (hook: ReturnType<typeof useCharacterChatDraft>) => void;
    }) {
        const hook = useCharacterChatDraft(characterId, {
            debounceMs: 300,
            storage: mockStorage,
        });
        onHook(hook);
        return createElement("div", null, hook.input);
    }

    it("initializes with saved draft from storage", () => {
        saveCharacterDraft("kiana", "Hello Kiana!", mockStorage);

        let currentHook!: ReturnType<typeof useCharacterChatDraft>;
        act(() => {
            root?.render(
                createElement(TestHarness, {
                    characterId: "kiana",
                    onHook: (h) => {
                        currentHook = h;
                    },
                })
            );
        });

        expect(currentHook.input).toBe("Hello Kiana!");
    });

    it("debounces saving to storage after 300ms", () => {
        let currentHook!: ReturnType<typeof useCharacterChatDraft>;
        act(() => {
            root?.render(
                createElement(TestHarness, {
                    characterId: "kiana",
                    onHook: (h) => {
                        currentHook = h;
                    },
                })
            );
        });

        act(() => {
            currentHook.setInput("Draft text");
        });

        // Immediately after input, storage has not been updated yet
        expect(loadSavedCharacterDraft("kiana", mockStorage)).toBe("");

        // Advance 200ms -> still not saved
        act(() => {
            vi.advanceTimersByTime(200);
        });
        expect(loadSavedCharacterDraft("kiana", mockStorage)).toBe("");

        // Advance another 100ms (total 300ms) -> now saved
        act(() => {
            vi.advanceTimersByTime(100);
        });
        expect(loadSavedCharacterDraft("kiana", mockStorage)).toBe("Draft text");
    });

    it("cancels previous debounce on rapid consecutive typing", () => {
        let currentHook!: ReturnType<typeof useCharacterChatDraft>;
        act(() => {
            root?.render(
                createElement(TestHarness, {
                    characterId: "kiana",
                    onHook: (h) => {
                        currentHook = h;
                    },
                })
            );
        });

        act(() => {
            currentHook.setInput("Step 1");
            vi.advanceTimersByTime(150);
            currentHook.setInput("Step 2");
            vi.advanceTimersByTime(150);
        });

        // 150ms after Step 2, storage should still not have Step 2
        expect(loadSavedCharacterDraft("kiana", mockStorage)).toBe("");

        // Advance remaining 150ms
        act(() => {
            vi.advanceTimersByTime(150);
        });
        expect(loadSavedCharacterDraft("kiana", mockStorage)).toBe("Step 2");
    });

    it("flushes in-flight draft of old character and loads new character draft on character switch", () => {
        saveCharacterDraft("bronya", "Bronya ready", mockStorage);

        let currentHook!: ReturnType<typeof useCharacterChatDraft>;
        act(() => {
            root?.render(
                createElement(TestHarness, {
                    characterId: "kiana",
                    onHook: (h) => {
                        currentHook = h;
                    },
                })
            );
        });

        act(() => {
            currentHook.setInput("Kiana in-flight draft");
            // Switch character immediately at 50ms (before 300ms debounce expires)
            vi.advanceTimersByTime(50);
            root?.render(
                createElement(TestHarness, {
                    characterId: "bronya",
                    onHook: (h) => {
                        currentHook = h;
                    },
                })
            );
        });

        // Assert: old character Kiana was flushed immediately
        expect(loadSavedCharacterDraft("kiana", mockStorage)).toBe("Kiana in-flight draft");
        // Assert: new character Bronya's draft is loaded into input
        expect(currentHook.input).toBe("Bronya ready");
    });

    it("immediately clears draft on clearDraft()", () => {
        saveCharacterDraft("kiana", "To be cleared", mockStorage);

        let currentHook!: ReturnType<typeof useCharacterChatDraft>;
        act(() => {
            root?.render(
                createElement(TestHarness, {
                    characterId: "kiana",
                    onHook: (h) => {
                        currentHook = h;
                    },
                })
            );
        });

        act(() => {
            currentHook.setInput("Unsaved edit");
            currentHook.clearDraft();
        });

        expect(currentHook.input).toBe("");
        expect(loadSavedCharacterDraft("kiana", mockStorage)).toBe("");

        // Timer expiring later should not resurrect the draft
        act(() => {
            vi.advanceTimersByTime(500);
        });
        expect(loadSavedCharacterDraft("kiana", mockStorage)).toBe("");
    });

    it("flushes in-flight draft on unmount", () => {
        let currentHook!: ReturnType<typeof useCharacterChatDraft>;
        act(() => {
            root?.render(
                createElement(TestHarness, {
                    characterId: "kiana",
                    onHook: (h) => {
                        currentHook = h;
                    },
                })
            );
        });

        act(() => {
            currentHook.setInput("Unmount test");
            vi.advanceTimersByTime(100);
        });

        // Unmount before 300ms
        act(() => {
            root?.unmount();
        });

        expect(loadSavedCharacterDraft("kiana", mockStorage)).toBe("Unmount test");
    });
});
