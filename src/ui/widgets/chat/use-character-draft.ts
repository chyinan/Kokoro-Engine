// pattern: Imperative Shell

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import {
    DEFAULT_CHAT_DRAFT_DEBOUNCE_MS,
    clearCharacterDraft,
    clearCharacterDraftImages,
    loadSavedCharacterDraft,
    loadSavedCharacterDraftImages,
    saveCharacterDraft,
    saveCharacterDraftImages,
    checkImageAccessible,
} from "./chat-draft-layout";

export interface UseCharacterChatDraftOptions {
    readonly debounceMs?: number;
    readonly storage?: Storage;
    readonly imageStorage?: Storage;
    readonly validateImage?: (url: string) => Promise<boolean>;
}

/** 发起上传请求时捕获的草稿上下文快照(请求代次)。 */
export type CharacterImageDraftContext = {
    readonly characterId: string;
    /** 角色代次:每次 characterId 变化 +1,用于剪枝等异步结果的陈旧性校验。 */
    readonly generation: number;
    /** 候选图片列表版本:任何实时列表变更 +1,用于陈旧性校验与清空墓碑比较。 */
    readonly listVersion: number;
};

export interface UseCharacterChatDraftResult {
    readonly input: string;
    readonly setInput: (value: string | ((prev: string) => string)) => void;
    readonly pendingImages: string[];
    readonly setPendingImages: (value: string[] | ((prev: string[]) => string[])) => void;
    readonly clearDraft: () => void;
    /** 仅清空图片草稿并记录墓碑(如关闭 vision 时),清空之前发起的在途上传随后将被丢弃。 */
    readonly clearDraftImages: () => void;
    /** 捕获当前草稿上下文(角色/代次/列表版本),供异步上传在首个 await 之前快照。 */
    readonly getImageDraftContext: () => CharacterImageDraftContext;
    /**
     * 迟到上传的唯一下落点。返回 false 表示因草稿已被清空而丢弃(调用方可提示用户)。
     * 仍为发起角色时追加实时列表(append-only,与用户期间的编辑可组合);
     * 角色已切换时仅写入发起角色自己的草稿存储,绝不触碰当前角色的实时列表。
     */
    readonly appendPendingImage: (context: CharacterImageDraftContext, url: string) => boolean;
    readonly flushDraft: () => void;
}

function haveSameImageList(a: readonly string[], b: readonly string[]): boolean {
    if (a.length !== b.length) return false;
    return a.every((url, index) => url === b[index]);
}

export function useCharacterChatDraft(
    characterId: string,
    options?: UseCharacterChatDraftOptions
): UseCharacterChatDraftResult {
    const debounceMs = options?.debounceMs ?? DEFAULT_CHAT_DRAFT_DEBOUNCE_MS;
    const storage = options?.storage;
    const imageStorage = options?.imageStorage ?? options?.storage;
    const validateImage = options?.validateImage ?? checkImageAccessible;

    const [input, setInputState] = useState<string>(() =>
        loadSavedCharacterDraft(characterId, storage)
    );
    const [pendingImages, setPendingImagesState] = useState<string[]>(() =>
        loadSavedCharacterDraftImages(characterId, imageStorage)
    );

    const inputRef = useRef(input);
    inputRef.current = input;
    const pendingImagesRef = useRef(pendingImages);
    pendingImagesRef.current = pendingImages;

    // activeCharacterIdRef 只在切换 layout effect 内翻转,与列表装载/代次递增保持 commit 后原子一致;
    // 渲染期不赋值,避免翻转与被动 effect 之间被 IPC 上传回调插入(见对抗性审查 B1)
    const activeCharacterIdRef = useRef(characterId);
    // 角色代次:每次 characterId 变化 +1,用于剪枝等异步结果的陈旧性校验
    const characterGenerationRef = useRef(0);
    // 候选图片列表版本:任何实时列表变更 +1(setPendingImages / clearDraft / clearDraftImages / 切换装载 / 剪枝应用)
    const imageListVersionRef = useRef(0);
    // 当前最新的异步图片校验请求 ID,每次发起校验分配新 ID;
    // 并在角色切换、草稿清空、组件卸载时递增使在途校验失效
    const latestValidationRequestIdRef = useRef(0);
    // clearDraft 墓碑:记录每个角色最后一次清空草稿时的列表版本,
    // 丢弃在清空之前发起、清空之后才返回的迟到上传,避免复活已清除的图片
    const clearedImageVersionsRef = useRef(new Map<string, number>());

    const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const imageDebounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Flushes pending changes for a given character to storage immediately
    const flushDraftFor = useCallback(
        (targetCharId: string, text: string, images: string[], force = false) => {
            if (debounceTimerRef.current !== null || force) {
                if (debounceTimerRef.current !== null) {
                    clearTimeout(debounceTimerRef.current);
                    debounceTimerRef.current = null;
                }
                saveCharacterDraft(targetCharId, text, storage);
            }
            if (imageDebounceTimerRef.current !== null || force) {
                if (imageDebounceTimerRef.current !== null) {
                    clearTimeout(imageDebounceTimerRef.current);
                    imageDebounceTimerRef.current = null;
                }
                saveCharacterDraftImages(targetCharId, images, imageStorage);
            }
        },
        [imageStorage, storage]
    );

    const validateAndPruneImages = useCallback(
        async (targetCharId: string, candidateImages: readonly string[]) => {
            if (candidateImages.length === 0) return;
            // 绑定本次校验唯一请求 ID,捕获候选图片不可变快照、角色代次与列表版本
            const requestId = ++latestValidationRequestIdRef.current;
            const candidateSnapshot = Object.freeze([...candidateImages]);
            const startGeneration = characterGenerationRef.current;
            const startVersion = imageListVersionRef.current;
            const results = await Promise.all(
                candidateSnapshot.map(async (url) => {
                    try {
                        return {
                            url,
                            valid: await validateImage(url),
                        };
                    } catch {
                        return { url, valid: false };
                    }
                })
            );
            // 应用前确认:
            // 1. 目标角色仍然处于激活状态
            if (activeCharacterIdRef.current !== targetCharId) return;
            // 2. 角色代次未发生变化(避免 A->B->A 往返应用)
            if (characterGenerationRef.current !== startGeneration) return;
            // 3. 当前校验请求仍是最新发起的请求(丢弃被后发校验覆盖的旧请求)
            if (latestValidationRequestIdRef.current !== requestId) return;
            // 4. 候选列表版本未发生变更(用户未在此期间增删或清空草稿)
            if (imageListVersionRef.current !== startVersion) return;
            // 5. 实时列表内容仍与发起校验时的快照完全一致
            if (!haveSameImageList(pendingImagesRef.current, candidateSnapshot)) return;

            const surviving = results.filter(r => r.valid).map(r => r.url);
            if (surviving.length !== candidateSnapshot.length) {
                imageListVersionRef.current += 1;
                pendingImagesRef.current = surviving;
                setPendingImagesState(surviving);
                saveCharacterDraftImages(targetCharId, surviving, imageStorage);
            }
        },
        [imageStorage, validateImage]
    );

    // Validate initial draft images on mount
    useEffect(() => {
        const initial = loadSavedCharacterDraftImages(characterId, imageStorage);
        if (initial.length > 0) {
            void validateAndPruneImages(characterId, initial);
        }
    }, [characterId, imageStorage, validateAndPruneImages]);

    const flushDraft = useCallback(() => {
        flushDraftFor(activeCharacterIdRef.current, inputRef.current, pendingImagesRef.current, true);
    }, [flushDraftFor]);

    // Handle character switching
    const prevCharacterIdRef = useRef(characterId);
    // 必须用 useLayoutEffect:角色 ID 翻转、旧角色 flush、新角色装载、代次/版本递增
    // 必须在 commit 内原子完成,否则被动 effect 之前的窗口可能被 IPC 上传回调插入,
    // 导致迟到上传写错角色或 flush 覆盖其存储写入(见对抗性审查 B1)
    useLayoutEffect(() => {
        if (prevCharacterIdRef.current !== characterId) {
            activeCharacterIdRef.current = characterId;
            characterGenerationRef.current += 1;
            latestValidationRequestIdRef.current += 1;

            // Flush old character's in-flight draft
            flushDraftFor(prevCharacterIdRef.current, inputRef.current, pendingImagesRef.current);

            // Load new character's draft
            const nextDraft = loadSavedCharacterDraft(characterId, storage);
            inputRef.current = nextDraft;
            setInputState(nextDraft);

            const nextImages = loadSavedCharacterDraftImages(characterId, imageStorage);
            imageListVersionRef.current += 1;
            pendingImagesRef.current = nextImages;
            setPendingImagesState(nextImages);
            if (nextImages.length > 0) {
                void validateAndPruneImages(characterId, nextImages);
            }

            prevCharacterIdRef.current = characterId;
        }
    }, [characterId, flushDraftFor, imageStorage, storage, validateAndPruneImages]);

    // Set input with debounced persistence (timer managed outside setState updater)
    const setInput = useCallback(
        (value: string | ((prev: string) => string)) => {
            if (debounceTimerRef.current !== null) {
                clearTimeout(debounceTimerRef.current);
                debounceTimerRef.current = null;
            }

            const next = typeof value === "function" ? value(inputRef.current) : value;
            inputRef.current = next;
            setInputState(next);

            const targetCharId = activeCharacterIdRef.current;
            debounceTimerRef.current = setTimeout(() => {
                debounceTimerRef.current = null;
                saveCharacterDraft(targetCharId, inputRef.current, storage);
            }, debounceMs);
        },
        [debounceMs, storage]
    );

    // Set pending images with debounced persistence
    const setPendingImages = useCallback(
        (value: string[] | ((prev: string[]) => string[])) => {
            if (imageDebounceTimerRef.current !== null) {
                clearTimeout(imageDebounceTimerRef.current);
                imageDebounceTimerRef.current = null;
            }

            const next = typeof value === "function" ? value(pendingImagesRef.current) : value;
            imageListVersionRef.current += 1;
            pendingImagesRef.current = next;
            setPendingImagesState(next);

            const targetCharId = activeCharacterIdRef.current;
            imageDebounceTimerRef.current = setTimeout(() => {
                imageDebounceTimerRef.current = null;
                saveCharacterDraftImages(targetCharId, pendingImagesRef.current, imageStorage);
            }, debounceMs);
        },
        [debounceMs, imageStorage]
    );

    // Clear draft immediately (called on submit or auto-send)
    const clearDraft = useCallback(() => {
        if (debounceTimerRef.current !== null) {
            clearTimeout(debounceTimerRef.current);
            debounceTimerRef.current = null;
        }
        if (imageDebounceTimerRef.current !== null) {
            clearTimeout(imageDebounceTimerRef.current);
            imageDebounceTimerRef.current = null;
        }
        // 记录墓碑:在清空之前发起、清空之后才返回的迟到上传将被丢弃,不复活已清除的图片
        imageListVersionRef.current += 1;
        latestValidationRequestIdRef.current += 1;
        clearedImageVersionsRef.current.set(activeCharacterIdRef.current, imageListVersionRef.current);
        clearCharacterDraft(activeCharacterIdRef.current, storage);
        clearCharacterDraftImages(activeCharacterIdRef.current, imageStorage);
        inputRef.current = "";
        setInputState("");
        pendingImagesRef.current = [];
        setPendingImagesState([]);
    }, [imageStorage, storage]);

    // 仅清空图片草稿并记录墓碑(如关闭 vision):在途上传清空后返回将被丢弃
    const clearDraftImages = useCallback(() => {
        if (imageDebounceTimerRef.current !== null) {
            clearTimeout(imageDebounceTimerRef.current);
            imageDebounceTimerRef.current = null;
        }
        imageListVersionRef.current += 1;
        latestValidationRequestIdRef.current += 1;
        clearedImageVersionsRef.current.set(activeCharacterIdRef.current, imageListVersionRef.current);
        clearCharacterDraftImages(activeCharacterIdRef.current, imageStorage);
        pendingImagesRef.current = [];
        setPendingImagesState([]);
    }, [imageStorage]);

    // 捕获当前草稿上下文(角色/代次/列表版本),供上传请求在首个 await 之前快照
    const getImageDraftContext = useCallback((): CharacterImageDraftContext => ({
        characterId: activeCharacterIdRef.current,
        generation: characterGenerationRef.current,
        listVersion: imageListVersionRef.current,
    }), []);

    // 迟到上传的唯一下落点:
    // 1. 发起角色在上传期间被清空(发送消息/关闭 vision)→ 丢弃并返回 false,调用方可提示用户
    // 2. 仍是发起角色 → 追加实时列表(append-only,与用户期间的编辑可组合)
    // 3. 角色已切换 → 仅写入发起角色自己的草稿存储,绝不触碰当前角色的实时列表
    // 注:组件卸载后 resolve 的死实例仍写入正确角色的存储是有意为之(图片归属不变)
    const appendPendingImage = useCallback(
        (context: CharacterImageDraftContext, url: string): boolean => {
            const clearedVersion = clearedImageVersionsRef.current.get(context.characterId);
            if (clearedVersion !== undefined && clearedVersion > context.listVersion) {
                console.warn(
                    "[useCharacterChatDraft] Dropping late image upload: draft cleared after upload started:",
                    context.characterId,
                );
                return false;
            }
            if (activeCharacterIdRef.current === context.characterId) {
                setPendingImages(prev => [...prev, url]);
                return true;
            }
            const saved = loadSavedCharacterDraftImages(context.characterId, imageStorage);
            saveCharacterDraftImages(context.characterId, [...saved, url], imageStorage);
            return true;
        },
        [imageStorage, setPendingImages]
    );

    // Flush on unmount or beforeunload
    useEffect(() => {
        const handleBeforeUnload = () => {
            flushDraftFor(activeCharacterIdRef.current, inputRef.current, pendingImagesRef.current);
        };

        if (typeof window !== "undefined") {
            window.addEventListener("beforeunload", handleBeforeUnload);
        }

        return () => {
            latestValidationRequestIdRef.current += 1;
            if (typeof window !== "undefined") {
                window.removeEventListener("beforeunload", handleBeforeUnload);
            }
            flushDraftFor(activeCharacterIdRef.current, inputRef.current, pendingImagesRef.current);
        };
    }, [flushDraftFor]);

    return {
        input,
        setInput,
        pendingImages,
        setPendingImages,
        clearDraft,
        clearDraftImages,
        getImageDraftContext,
        appendPendingImage,
        flushDraft,
    };
}
