// @vitest-environment jsdom
// pattern: Imperative Shell

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
    loadSavedCharacterDraft,
    loadSavedCharacterDraftImages,
    saveCharacterDraft,
    saveCharacterDraftImages,
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

    describe("image draft character isolation", () => {
        it("initializes with saved image draft from storage", () => {
            saveCharacterDraftImages("kiana", ["http://test/1.png"], mockStorage);

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

            expect(currentHook.pendingImages).toEqual(["http://test/1.png"]);
        });

        it("debounces saving pendingImages to storage after 300ms", () => {
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
                currentHook.setPendingImages(["http://test/new.png"]);
            });

            // Not saved yet before debounce expires
            expect(loadSavedCharacterDraftImages("kiana", mockStorage)).toEqual([]);

            // Advance timers by 300ms
            act(() => {
                vi.advanceTimersByTime(300);
            });

            expect(loadSavedCharacterDraftImages("kiana", mockStorage)).toEqual(["http://test/new.png"]);
        });

        it("switches character and isolates both text and image drafts", () => {
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
                currentHook.setInput("Kiana's text");
                currentHook.setPendingImages(["http://test/kiana.png"]);
            });

            // Switch to bronya
            act(() => {
                root?.render(
                    createElement(TestHarness, {
                        characterId: "bronya",
                        onHook: (h) => {
                            currentHook = h;
                        },
                    })
                );
            });

            // Bronya should start empty
            expect(currentHook.input).toBe("");
            expect(currentHook.pendingImages).toEqual([]);

            // Set Bronya's draft
            act(() => {
                currentHook.setInput("Bronya's text");
                currentHook.setPendingImages(["http://test/bronya.png"]);
            });

            // Switch back to kiana
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

            // Kiana's text and images should be preserved
            expect(currentHook.input).toBe("Kiana's text");
            expect(currentHook.pendingImages).toEqual(["http://test/kiana.png"]);
        });

        it("clears both text and image drafts atomically on clearDraft()", () => {
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
                currentHook.setInput("Will be cleared");
                currentHook.setPendingImages(["http://test/clear.png"]);
                vi.advanceTimersByTime(300);
            });

            expect(loadSavedCharacterDraft("kiana", mockStorage)).toBe("Will be cleared");
            expect(loadSavedCharacterDraftImages("kiana", mockStorage)).toEqual(["http://test/clear.png"]);

            act(() => {
                currentHook.clearDraft();
            });

            expect(currentHook.input).toBe("");
            expect(currentHook.pendingImages).toEqual([]);
            expect(loadSavedCharacterDraft("kiana", mockStorage)).toBe("");
            expect(loadSavedCharacterDraftImages("kiana", mockStorage)).toEqual([]);
        });

        it("automatically prunes dead images on mount when validateImage returns false", async () => {
            saveCharacterDraftImages("kiana", ["http://test/alive.png", "http://test/dead.png"], mockStorage);

            let currentHook!: ReturnType<typeof useCharacterChatDraft>;
            await act(async () => {
                root?.render(
                    createElement(function CustomHarness() {
                        const hook = useCharacterChatDraft("kiana", {
                            storage: mockStorage,
                            validateImage: async (url) => url.includes("alive"),
                        });
                        currentHook = hook;
                        return null;
                    })
                );
                // Allow async validation Promise to resolve and re-render
                await Promise.resolve();
            });

            expect(currentHook.pendingImages).toEqual(["http://test/alive.png"]);
            expect(loadSavedCharacterDraftImages("kiana", mockStorage)).toEqual(["http://test/alive.png"]);
        });

        it("automatically prunes dead images on character switch when validateImage returns false", async () => {
            saveCharacterDraftImages("bronya", ["http://test/expired.png"], mockStorage);

            let currentHook!: ReturnType<typeof useCharacterChatDraft>;
            await act(async () => {
                root?.render(
                    createElement(function SwitchHarness({ charId }: { charId: string }) {
                        const hook = useCharacterChatDraft(charId, {
                            storage: mockStorage,
                            validateImage: async (url) => !url.includes("expired"),
                        });
                        currentHook = hook;
                        return null;
                    }, { charId: "kiana" })
                );
                await Promise.resolve();
            });

            expect(currentHook.pendingImages).toEqual([]);

            // Switch to bronya who had an expired image
            await act(async () => {
                root?.render(
                    createElement(function SwitchHarness({ charId }: { charId: string }) {
                        const hook = useCharacterChatDraft(charId, {
                            storage: mockStorage,
                            validateImage: async (url) => !url.includes("expired"),
                        });
                        currentHook = hook;
                        return null;
                    }, { charId: "bronya" })
                );
                await Promise.resolve();
            });

            expect(currentHook.pendingImages).toEqual([]);
            expect(loadSavedCharacterDraftImages("bronya", mockStorage)).toEqual([]);
        });

        it("supports separate imageStorage from text storage", () => {
            const textStore: Record<string, string> = {};
            const textStorage = {
                getItem: (k: string) => textStore[k] ?? null,
                setItem: (k: string, v: string) => { textStore[k] = v; },
                removeItem: (k: string) => { delete textStore[k]; },
            } as unknown as Storage;

            const imageStore: Record<string, string> = {};
            const imageStorage = {
                getItem: (k: string) => imageStore[k] ?? null,
                setItem: (k: string, v: string) => { imageStore[k] = v; },
                removeItem: (k: string) => { delete imageStore[k]; },
            } as unknown as Storage;

            let currentHook!: ReturnType<typeof useCharacterChatDraft>;
            act(() => {
                root?.render(
                    createElement(function StorageHarness() {
                        const hook = useCharacterChatDraft("kiana", {
                            storage: textStorage,
                            imageStorage,
                            debounceMs: 100,
                        });
                        currentHook = hook;
                        return null;
                    })
                );
            });

            act(() => {
                currentHook.setInput("text in textStorage");
                currentHook.setPendingImages(["http://test/img.png"]);
                vi.advanceTimersByTime(100);
            });

            expect(loadSavedCharacterDraft("kiana", textStorage)).toBe("text in textStorage");
            expect(loadSavedCharacterDraftImages("kiana", textStorage)).toEqual([]);
            expect(loadSavedCharacterDraftImages("kiana", imageStorage)).toEqual(["http://test/img.png"]);
        });
    });
});
