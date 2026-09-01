// pattern: Imperative Shell
/**
 * CharacterManager — Persona tab replacement
 *
 * Full character management UI: list, create, edit, delete,
 * and import SillyTavern character cards (JSON / PNG).
 */
import { useState, useEffect, useCallback, useRef, type ChangeEvent } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { Plus, Upload, Trash2, UserCircle, Check, X, User } from "lucide-react";
import { characterDb } from "../../lib/db";
import { parseCharacterCard } from "../../lib/character-card-parser";
import { getKokoroErrorMessage, setUserName, setUserPersona, getProactiveEnabled, listCharacters, createCharacter, createCharacterWithAvatar, updateCharacter, updateCharacterWithAvatar, deleteCharacter } from "../../lib/kokoro-bridge";
import type { CharacterRecord } from "../../lib/kokoro-bridge";
import {
    readCharacterRuntimeProfile,
    type CharacterRuntimeOverrides,
} from "../../features/characters/character-runtime-overrides";
import { normalizeCharacterPersona } from "./character-persona";
import { Languages, MessageCircle } from "lucide-react";
import { Select } from "@/components/ui/select";
import { useTranslation, Trans } from "react-i18next";

export const RESPONSE_LANGUAGE_PRESETS = ["日本語", "English", "中文", "繁體中文", "한국어", "Русский"] as const;
export const USER_LANGUAGE_PRESETS = ["中文", "繁體中文", "English", "日本語", "한국어", "Русский"] as const;

const LANGUAGE_CODE_TO_DISPLAY: Readonly<Record<string, string>> = {
    en: "English",
    "en-us": "English",
    ja: "日本語",
    "ja-jp": "日本語",
    zh: "中文",
    "zh-cn": "中文",
    "zh-tw": "繁體中文",
    "zh-hant": "繁體中文",
    ko: "한국어",
    "ko-kr": "한국어",
    ru: "Русский",
    "ru-ru": "Русский",
};

function normalizeLanguageValue(value: string): string {
    return LANGUAGE_CODE_TO_DISPLAY[value.trim().toLowerCase()] ?? value;
}

export function getLanguageSelectValue(value: string, presets: readonly string[]) {
    const normalizedValue = normalizeLanguageValue(value);
    if (normalizedValue === "" || normalizedValue === "auto") {
        return "auto";
    }

    return normalizedValue === "__custom__" || presets.includes(normalizedValue) ? normalizedValue : "__custom__";
}

export function shouldShowCustomLanguageInput(value: string, presets: readonly string[]) {
    const normalizedValue = normalizeLanguageValue(value);
    return normalizedValue === "__custom__" || (normalizedValue !== "" && normalizedValue !== "auto" && !presets.includes(normalizedValue));
}

function getCustomLanguageInputValue(value: string) {
    return value === "__custom__" ? "" : value;
}

function sanitizeCustomLanguageValue(value: string) {
    return value === "__custom__" ? "" : value;
}

// ── Shared style tokens (matching SettingsPanel) ───

const inputClasses = clsx(
    "w-full bg-black/40 border border-[var(--color-border)]",
    "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
    "rounded-md px-4 py-3 text-sm",
    "focus:outline-none focus:border-[var(--color-accent)] focus:shadow-[var(--glow-accent)]",
    "transition-all font-body"
);

const labelClasses = "block text-xs font-heading font-semibold tracking-wider uppercase text-[var(--color-text-secondary)] mb-2";

// ── Default character (always present, cannot be deleted) ─

const DEFAULT_PERSONA = "You are a friendly, warm companion character. Respond with personality and emotion.";

function makeDefaultCharacter(): CharacterRecord {
    return {
        id: crypto.randomUUID(),
        name: "Kokoro",
        persona: DEFAULT_PERSONA,
        user_nickname: "User",
        source_format: "manual",
        created_at: 0,
        updated_at: 0,
    };
}

// ── User profile storage ───────────────────────────

const USER_NAME_KEY = "kokoro_user_name";
const USER_PERSONA_KEY = "kokoro_user_persona";

export interface UserProfile {
    name: string;
    persona: string;
}

function loadUserProfile(): UserProfile {
    return {
        name: localStorage.getItem(USER_NAME_KEY) || "User",
        persona: localStorage.getItem(USER_PERSONA_KEY) || "",
    };
}

function normalizeCharacterRecord(character: CharacterRecord, userProfile: UserProfile): CharacterRecord {
    const persona = normalizeCharacterPersona(
        character.persona,
        { name: character.name, userNickname: character.user_nickname },
        userProfile.name,
        userProfile.persona,
    );
    return persona === character.persona ? character : { ...character, persona };
}

function saveUserProfile(profile: UserProfile) {
    localStorage.setItem(USER_NAME_KEY, profile.name);
    localStorage.setItem(USER_PERSONA_KEY, profile.persona);
}

type ImportFeedback = {
    readonly kind: "success" | "error";
    readonly message: string;
};

// ── Storage key for active character ───────────────

const ACTIVE_CHAR_KEY = "kokoro_active_character_id";

// ── Props ──────────────────────────────────────────

interface CharacterManagerProps {
    /** Routes every user-initiated selection through the activation transaction owner. */
    onActivateCharacter: (characterId: string) => Promise<void>;
    onCharacterRuntimeChange: (overrides: Readonly<CharacterRuntimeOverrides>) => Promise<void>;
    onCharactersChanged?: (characters: ReadonlyArray<CharacterRecord>) => void;
    characters?: ReadonlyArray<CharacterRecord>;
    resolveAvatarUrl?: (path: string) => string;
    characterToEditId?: string | null;
    activeCharacterId?: string;
    /** Current response language setting */
    responseLanguage: string;
    /** Called when the response language dropdown changes */
    onResponseLanguageChange: (lang: string) => void;
    /** Current user language setting (for inline translation) */
    userLanguage: string;
    /** Called when the user language dropdown changes */
    onUserLanguageChange: (lang: string) => void;
}

// ── Component ──────────────────────────────────────

export default function CharacterManager({ onActivateCharacter, onCharacterRuntimeChange, onCharactersChanged, characters: charactersProp, resolveAvatarUrl, characterToEditId, activeCharacterId: activeCharacterIdProp, responseLanguage, onResponseLanguageChange, userLanguage, onUserLanguageChange }: CharacterManagerProps) {
    const { t } = useTranslation();
    const [characters, setCharacters] = useState<CharacterRecord[]>([]);
    const [activeId, setActiveId] = useState<string | null>(null);
    const [editChar, setEditChar] = useState<CharacterRecord | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
    const [importFeedback, setImportFeedback] = useState<ImportFeedback | null>(null);
    const avatarInputRef = useRef<HTMLInputElement | null>(null);
    const [isAvatarUpdating, setIsAvatarUpdating] = useState(false);
    const [userProfile, setUserProfile] = useState<UserProfile>(loadUserProfile);
    const [proactiveEnabled, setProactiveEnabledState] = useState(true);

    const onActivateCharacterRef = useRef(onActivateCharacter);
    onActivateCharacterRef.current = onActivateCharacter;
    const onCharactersChangedRef = useRef(onCharactersChanged);
    onCharactersChangedRef.current = onCharactersChanged;

    function publishCharacters(nextCharacters: Array<CharacterRecord>): void {
        setCharacters(nextCharacters);
        onCharactersChangedRef.current?.(nextCharacters);
    }

    useEffect(() => {
        if (charactersProp === undefined) return;
        setCharacters(Array.from(charactersProp));
    }, [charactersProp]);

    useEffect(() => {
        getProactiveEnabled().then(setProactiveEnabledState).catch(() => {});
    }, []);

    const loadCharacters = useCallback(async () => {
        setIsLoading(true);
        try {
            // Migration/restore: copy IndexedDB characters to SQLite (upsert), then delete from IndexedDB
            const idbChars = await characterDb.getAll();
            for (const c of idbChars) {
                if (!c.stableId) continue;
                const record = {
                    id: c.stableId,
                    name: c.name,
                    persona: c.persona,
                    user_nickname: c.userNickname,
                    source_format: c.sourceFormat ?? "manual",
                    created_at: c.createdAt ?? 0,
                    updated_at: c.updatedAt ?? 0,
                };
                await createCharacter(record).catch(() => {});
                // Always update to ensure backup-restored data overwrites stale SQLite records
                await updateCharacter(record).catch(() => {});
                if (c.id != null) {
                    await characterDb.remove(c.id).catch(() => {});
                }
            }

            const currentUserProfile = loadUserProfile();
            let all = await listCharacters();
            const normalizedCharacters = all.map((character) => normalizeCharacterRecord(character, currentUserProfile));
            const normalizedCharacterIds = new Set<string>();
            for (const character of normalizedCharacters) {
                const previous = all.find((candidate) => candidate.id === character.id);
                if (previous?.persona === character.persona) continue;
                normalizedCharacterIds.add(character.id);
                await updateCharacter(character).catch(() => {});
            }
            all = normalizedCharacters;
            setUserName(currentUserProfile.name).catch(() => {});

            if (all.length === 0) {
                const defaultChar = makeDefaultCharacter();
                await createCharacter(defaultChar);
                all = [defaultChar];
            }

            publishCharacters(all);

            const savedId = localStorage.getItem(ACTIVE_CHAR_KEY);
            const active = (savedId && all.find(c => c.id === savedId)) ? all.find(c => c.id === savedId)! : all[0];
            setActiveId(active.id);
            setEditChar(active);
            if (normalizedCharacterIds.has(active.id)) {
                await onActivateCharacterRef.current(active.id).catch((error) => {
                    console.error("[CharacterManager] Failed to refresh normalized character runtime:", error);
                });
            }
        } catch (err) {
            console.error("[CharacterManager] Failed to load characters:", err);
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        loadCharacters();
    }, [loadCharacters]);

    useEffect(() => {
        if (characterToEditId === null || characterToEditId === undefined) return;
        const selected = characters.find((character) => character.id === characterToEditId);
        if (selected) setEditChar({ ...selected });
    }, [characterToEditId, characters]);

    useEffect(() => {
        if (!activeCharacterIdProp || characterToEditId) return;
        const active = characters.find((character) => character.id === activeCharacterIdProp);
        if (!active) return;
        setActiveId(active.id);
        setEditChar({ ...active });
    }, [activeCharacterIdProp, characterToEditId, characters, userProfile]);

    const selectCharacter = async (char: CharacterRecord) => {
        try {
            await onActivateCharacter(char.id);
            setActiveId(char.id);
            setEditChar({ ...char });
            setProactiveEnabledState(
                readCharacterRuntimeProfile(char.runtime_profile_json).proactive_enabled ?? true,
            );
            setConfirmDeleteId(null);
        } catch (error) {
            console.error("[CharacterManager] Failed to activate character:", error);
        }
    };

    const handleCreate = async () => {
        const now = Date.now();
        const newChar: CharacterRecord = {
            id: crypto.randomUUID(),
            name: "New Character",
            persona: "",
            user_nickname: "User",
            source_format: "manual",
            created_at: now,
            updated_at: now,
        };
        try {
            await createCharacter(newChar);
            publishCharacters([...characters, newChar]);
            await selectCharacter(newChar);
        } catch (err) {
            console.error("[CharacterManager] Failed to create character:", err);
        }
    };

    const handleFieldChange = (field: keyof CharacterRecord, value: string) => {
        if (!editChar) return;
        setEditChar(prev => prev ? { ...prev, [field]: value } : null);
    };

    const handleAvatarFileChange = async (event: ChangeEvent<HTMLInputElement>) => {
        if (isAvatarUpdating) return;
        const file = event.currentTarget.files?.[0];
        event.currentTarget.value = "";
        if (!file || !editChar) return;
        setIsAvatarUpdating(true);
        try {
            const updated: CharacterRecord = {
                ...editChar,
                avatar_path: `character-instance-resource://${editChar.id}/avatar.png`,
                updated_at: Date.now(),
            };
            await updateCharacterWithAvatar(updated, new Uint8Array(await file.arrayBuffer()));
            publishCharacters(characters.map(character => character.id === updated.id ? updated : character));
            setEditChar(updated);
            setImportFeedback({
                kind: "success",
                message: t("settings.persona.status.avatar_updated"),
            });
        } catch (error) {
            setImportFeedback({
                kind: "error",
                message: t("settings.persona.status.avatar_update_failed", { error: getKokoroErrorMessage(error) }),
            });
        } finally {
            setIsAvatarUpdating(false);
        }
    };

    const handleRemoveAvatar = async () => {
        if (!editChar?.avatar_path || isAvatarUpdating) return;
        setIsAvatarUpdating(true);
        try {
            const updated: CharacterRecord = {
                ...editChar,
                avatar_path: null,
                updated_at: Date.now(),
            };
            await updateCharacter(updated);
            publishCharacters(characters.map(character => character.id === updated.id ? updated : character));
            setEditChar(updated);
            setImportFeedback({
                kind: "success",
                message: t("settings.persona.status.avatar_removed"),
            });
        } catch (error) {
            setImportFeedback({
                kind: "error",
                message: t("settings.persona.status.avatar_update_failed", { error: getKokoroErrorMessage(error) }),
            });
        } finally {
            setIsAvatarUpdating(false);
        }
    };

    const handleSaveEdit = async () => {
        if (!editChar) return;
        try {
            const updated = {
                ...normalizeCharacterRecord(editChar, userProfile),
                updated_at: Date.now(),
            };
            await updateCharacter(updated);
            publishCharacters(characters.map(c => c.id === updated.id ? updated : c));
            setEditChar(updated);
            if (activeId === updated.id) {
                await onActivateCharacter(updated.id);
            }
        } catch (err) {
            console.error("[CharacterManager] Failed to update character:", err);
        }
    };

    const handleDelete = async (charId: string) => {
        try {
            await deleteCharacter(charId);
            const remaining = characters.filter(c => c.id !== charId);
            publishCharacters(remaining);
            setConfirmDeleteId(null);

            if (activeId === charId || editChar?.id === charId) {
                if (remaining.length > 0) {
                    await selectCharacter(remaining[0]);
                } else {
                    const defaultChar = makeDefaultCharacter();
                    await createCharacter(defaultChar);
                    publishCharacters([defaultChar]);
                    await selectCharacter(defaultChar);
                }
            }
        } catch (err) {
            console.error("[CharacterManager] Failed to delete character:", err);
        }
    };

    const handleImport = async () => {
        const input = document.createElement("input");
        input.type = "file";
        input.accept = ".json,.png";
        input.onchange = async (e) => {
            const file = (e.target as HTMLInputElement).files?.[0];
            if (!file) return;
            try {
                const profile = await parseCharacterCard(file);
                const now = Date.now();
                const id = crypto.randomUUID();
                const avatarPath = profile.avatar_bytes
                    ? `character-instance-resource://${id}/avatar.png`
                    : profile.avatar_path;
                const newChar = {
                    id,
                    name: profile.name,
                    persona: profile.persona,
                    description: profile.description,
                    avatar_path: avatarPath,
                    greeting: profile.greeting,
                    example_dialogue: profile.example_dialogue,
                    user_nickname: profile.user_nickname,
                    source_format: profile.source_format ?? "manual",
                    created_at: now,
                    updated_at: now,
                };
                if (profile.avatar_bytes) {
                    await createCharacterWithAvatar(newChar, profile.avatar_bytes);
                } else {
                    await createCharacter(newChar);
                }
                publishCharacters([...characters, newChar]);
                await selectCharacter(newChar);
                setImportFeedback({
                    kind: "success",
                    message: t("settings.persona.status.imported", { name: profile.name }),
                });
                setTimeout(() => setImportFeedback(null), 3000);
            } catch (err) {
                console.error("[CharacterManager] Import failed:", err);
                setImportFeedback({
                    kind: "error",
                    message: t("settings.persona.status.import_failed", { error: getKokoroErrorMessage(err) }),
                });
                setTimeout(() => setImportFeedback(null), 5000);
            }
        };
        input.click();
    };

    // ── Render ─────────────────────────────────────

    if (isLoading) {
        return (
            <div className="flex items-center justify-center py-12 text-[var(--color-text-muted)] text-sm">
                {t("settings.persona.list.loading")}
            </div>
        );
    }

    // ── User profile handlers ──────────────────────

    const handleUserProfileChange = (field: keyof UserProfile, value: string) => {
        setUserProfile(prev => ({ ...prev, [field]: value }));
    };

    const handleUserProfileSave = () => {
        saveUserProfile(userProfile);
        setUserName(userProfile.name).catch(e => console.error("[CharacterManager] Failed to set user name:", e));
        setUserPersona(userProfile.persona).catch(e => console.error("[CharacterManager] Failed to persist user profile:", e));
        // Keep the persisted character persona raw; activation composes the runtime prompt.
        if (editChar) {
            const nextPersona = normalizeCharacterRecord(editChar, userProfile).persona;
            void onCharacterRuntimeChange({ persona: nextPersona }).catch(e => console.error("[CharacterManager] Failed to set persona:", e));
        }
    };

    return (
        <div className="space-y-4">
            {/* ── User Profile ── */}
            <div>
                <label className={labelClasses}>
                    <User size={12} strokeWidth={2} className="inline-block mr-1.5 -mt-0.5" />
                    {t("settings.persona.user_profile.title")}
                </label>
                <p className="text-[10px] text-[var(--color-text-muted)] mb-3 -mt-1">
                    <Trans i18nKey="settings.persona.user_profile.desc" />
                </p>
                <div className="space-y-2">
                    <input
                        type="text"
                        value={userProfile.name}
                        onChange={e => handleUserProfileChange("name", e.target.value)}
                        onBlur={handleUserProfileSave}
                        placeholder={t("settings.persona.user_profile.name_placeholder")}
                        className={inputClasses}
                    />
                    <textarea
                        value={userProfile.persona}
                        onChange={e => handleUserProfileChange("persona", e.target.value)}
                        onBlur={handleUserProfileSave}
                        placeholder={t("settings.persona.user_profile.persona_placeholder")}
                        rows={3}
                        className={clsx(inputClasses, "resize-y min-h-[60px]")}
                    />
                </div>
            </div>

            {/* ── Response Language ── */}
            <div>
                <label className={labelClasses}>
                    <Languages size={12} strokeWidth={2} className="inline-block mr-1.5 -mt-0.5" />
                    {t("settings.persona.response_lang.label")}
                </label>
                <p className="text-[10px] text-[var(--color-text-muted)] mb-3 -mt-1">
                    {t("settings.persona.response_lang.desc")}
                </p>
                <Select
                    value={getLanguageSelectValue(responseLanguage || "", RESPONSE_LANGUAGE_PRESETS)}
                    onChange={v => {
                        if (v === "auto") onResponseLanguageChange("");
                        else if (v === "__custom__") onResponseLanguageChange("__custom__");
                        else onResponseLanguageChange(v);
                    }}
                    options={[
                        { value: "auto", label: t("settings.persona.response_lang.auto") },
                        { value: "日本語", label: "日本語 (Japanese)" },
                        { value: "English", label: "English" },
                        { value: "中文", label: "中文 (Simplified Chinese)" },
                        { value: "繁體中文", label: "繁體中文 (Traditional Chinese)" },
                        { value: "한국어", label: "한국어 (Korean)" },
                        { value: "Русский", label: "Русский (Russian)" },
                        { value: "__custom__", label: t("settings.persona.user_lang.custom") },
                    ]}
                />
                {/* Show custom input when language is not in presets */}
                {shouldShowCustomLanguageInput(responseLanguage, RESPONSE_LANGUAGE_PRESETS) && (
                    <input
                        type="text"
                        value={getCustomLanguageInputValue(responseLanguage)}
                        onChange={e => onResponseLanguageChange(sanitizeCustomLanguageValue(e.target.value))}
                        placeholder={t("settings.persona.response_lang.placeholder")}
                        className={clsx(inputClasses, "mt-2")}
                    />
                )}
            </div>

            {/* ── User Language (Translation) ── */}
            <div>
                <label className={labelClasses}>
                    <Languages size={12} strokeWidth={2} className="inline-block mr-1.5 -mt-0.5" />
                    {t("settings.persona.user_lang.label")}
                </label>
                <p className="text-[10px] text-[var(--color-text-muted)] mb-3 -mt-1">
                    {t("settings.persona.user_lang.desc")}
                </p>
                <Select
                    value={getLanguageSelectValue(userLanguage || "", USER_LANGUAGE_PRESETS)}
                    onChange={v => {
                        if (v === "auto") onUserLanguageChange("");
                        else if (v === "__custom__") onUserLanguageChange("__custom__");
                        else onUserLanguageChange(v);
                    }}
                    options={[
                        { value: "auto", label: t("settings.persona.user_lang.auto") },
                        { value: "中文", label: "中文 (Simplified Chinese)" },
                        { value: "繁體中文", label: "繁體中文 (Traditional Chinese)" },
                        { value: "English", label: "English" },
                        { value: "日本語", label: "日本語 (Japanese)" },
                        { value: "한국어", label: "한국어 (Korean)" },
                        { value: "Русский", label: "Русский (Russian)" },
                        { value: "__custom__", label: t("settings.persona.user_lang.custom") },
                    ]}
                />
                {shouldShowCustomLanguageInput(userLanguage, USER_LANGUAGE_PRESETS) && (
                    <input
                        type="text"
                        value={getCustomLanguageInputValue(userLanguage)}
                        onChange={e => onUserLanguageChange(sanitizeCustomLanguageValue(e.target.value))}
                        placeholder={t("settings.persona.response_lang.placeholder")}
                        className={clsx(inputClasses, "mt-2")}
                    />
                )}
            </div>

            {/* ── Proactive Messages (Idle Auto-Talk) ── */}
            <div>
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                        <label className={labelClasses}>
                            <MessageCircle size={12} strokeWidth={2} className="inline-block mr-1.5 -mt-0.5" />
                            {t("settings.persona.proactive.label")}
                        </label>
                        <p className="text-[10px] text-[var(--color-text-muted)] -mt-1">
                            {t("settings.persona.proactive.desc")}
                        </p>
                    </div>
                    <button
                        type="button"
                        aria-pressed={proactiveEnabled}
                        onClick={() => {
                            const next = !proactiveEnabled;
                            void onCharacterRuntimeChange({ proactiveEnabled: next })
                                .then(() => setProactiveEnabledState(next))
                                .catch(e => console.error("[CharacterManager] Failed to set proactive:", e));
                        }}
                        className={clsx(
                            "relative inline-flex h-6 w-11 items-center rounded-full transition-colors shrink-0",
                            proactiveEnabled ? "bg-[var(--color-accent)]" : "bg-[var(--color-border)]"
                        )}
                    >
                        <span
                            className={clsx(
                                "inline-block h-4 w-4 rounded-full bg-white transition-transform",
                                proactiveEnabled ? "translate-x-6" : "translate-x-1"
                            )}
                        />
                    </button>
                </div>
            </div>

            {/* ── Divider ── */}
            <div className="border-t border-[var(--color-border)]" />

            {/* ── Header row: label + action buttons ── */}
            <div className="flex items-center justify-between">
                <label className={labelClasses.replace("mb-2", "mb-0")}>{t("settings.persona.list.label")}</label>
                <div className="flex gap-2">
                    <motion.button
                        whileHover={{ scale: 1.05 }}
                        whileTap={{ scale: 0.95 }}
                        onClick={handleCreate}
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[10px] font-heading font-semibold tracking-wider uppercase border border-[var(--color-border)] text-[var(--color-text-secondary)] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors"
                    >
                        <Plus size={12} strokeWidth={2} />
                        {t("settings.persona.list.new")}
                    </motion.button>
                    <motion.button
                        whileHover={{ scale: 1.05 }}
                        whileTap={{ scale: 0.95 }}
                        onClick={handleImport}
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[10px] font-heading font-semibold tracking-wider uppercase border border-[var(--color-border)] text-[var(--color-text-secondary)] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors"
                    >
                        <Upload size={12} strokeWidth={2} />
                        {t("settings.persona.list.import")}
                    </motion.button>
                </div>
            </div>

            {/* ── Import feedback ── */}
            <AnimatePresence>
                {importFeedback && (
                    <motion.div
                        initial={{ opacity: 0, y: -8 }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={{ opacity: 0, y: -8 }}
                        className={clsx(
                            "text-xs px-3 py-2 rounded-md",
                            importFeedback.kind === "error"
                                ? "bg-[var(--color-error)]/10 text-[var(--color-error)]"
                                : "bg-[var(--color-accent-subtle)] text-[var(--color-accent)]"
                        )}
                    >
                        {importFeedback.message}
                    </motion.div>
                )}
            </AnimatePresence>

            {/* ── Character list ── */}
            <div className="bg-black/30 border border-[var(--color-border)] rounded-lg overflow-hidden max-h-[180px] overflow-y-auto scrollable">
                {characters.map(char => (
                    <div key={char.id} className="group relative">
                        {/* Confirm-delete overlay */}
                        <AnimatePresence>
                            {confirmDeleteId === char.id && (
                                <motion.div
                                    initial={{ opacity: 0 }}
                                    animate={{ opacity: 1 }}
                                    exit={{ opacity: 0 }}
                                    className="absolute inset-0 z-10 flex items-center justify-between px-4 bg-black/80 backdrop-blur-sm"
                                >
                                    <span className="text-[11px] text-[var(--color-error)] truncate">
                                        {t("settings.persona.list.delete_confirm", { name: char.name })}
                                    </span>
                                    <div className="flex gap-1.5 shrink-0">
                                        <motion.button
                                            whileHover={{ scale: 1.1 }}
                                            whileTap={{ scale: 0.9 }}
                                            onClick={() => handleDelete(char.id)}
                                            className="px-2.5 py-1 rounded text-[10px] font-heading font-semibold uppercase bg-[var(--color-error)]/20 text-[var(--color-error)] hover:bg-[var(--color-error)]/30 transition-colors"
                                        >
                                            {t("settings.persona.list.delete")}
                                        </motion.button>
                                        <motion.button
                                            whileHover={{ scale: 1.1 }}
                                            whileTap={{ scale: 0.9 }}
                                            onClick={() => setConfirmDeleteId(null)}
                                            className="p-1 rounded text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors"
                                        >
                                            <X size={14} strokeWidth={2} />
                                        </motion.button>
                                    </div>
                                </motion.div>
                            )}
                        </AnimatePresence>

                        {/* Row */}
                        <button
                            onClick={() => selectCharacter(char)}
                            className="w-full flex items-center gap-3 px-4 py-2.5 text-left text-[var(--color-text-secondary)] hover:bg-white/5 transition-colors"
                        >
                            <AvatarPreview
                                path={char.avatar_path ?? null}
                                resolveAvatarUrl={resolveAvatarUrl}
                                version={char.updated_at}
                                alt={char.name}
                                className="h-9 w-9 shrink-0 rounded-full"
                            />
                            <div className="flex-1 min-w-0">
                                <span className="text-sm font-heading font-semibold tracking-wide truncate block">
                                    {char.name}
                                </span>
                                {char.source_format && char.source_format !== "manual" && (
                                    <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wider">
                                        {char.source_format}
                                    </span>
                                )}
                            </div>
                            {activeId === char.id && (
                                <Check size={14} strokeWidth={2} className="text-[var(--color-accent)] shrink-0" />
                            )}
                            <motion.div
                                whileHover={{ scale: 1.15 }}
                                whileTap={{ scale: 0.9 }}
                                onClick={(e) => {
                                    e.stopPropagation();
                                    setConfirmDeleteId(char.id);
                                }}
                                className="shrink-0 p-1 rounded opacity-0 group-hover:opacity-100 text-[var(--color-text-muted)] hover:text-[var(--color-error)] transition-all cursor-pointer"
                            >
                                <Trash2 size={13} strokeWidth={1.5} />
                            </motion.div>
                        </button>
                    </div>
                ))}
            </div>

            {/* ── Edit form ── */}
            {editChar && (
                <div className="space-y-3">
                    <div className="border-t border-[var(--color-border)] pt-4">
                        <label className={labelClasses}>{t("settings.persona.edit.title")}</label>
                    </div>

                    {/* Avatar */}
                    <div>
                        <label className="block text-[10px] font-heading font-semibold tracking-wider uppercase text-[var(--color-text-muted)] mb-1">
                            {t("settings.persona.edit.avatar")}
                        </label>
                        <div className="flex items-center gap-3 rounded-md border border-[var(--color-border)] bg-black/20 p-3">
                            <AvatarPreview
                                path={editChar.avatar_path ?? null}
                                resolveAvatarUrl={resolveAvatarUrl}
                                version={editChar.updated_at}
                                alt={editChar.name}
                                className="h-16 w-16 shrink-0 rounded-xl"
                            />
                            <div className="min-w-0 flex-1">
                                <input
                                    ref={avatarInputRef}
                                    type="file"
                                    accept=".png,image/png"
                                    onChange={handleAvatarFileChange}
                                    disabled={isAvatarUpdating}
                                    className="hidden"
                                />
                                <div className="flex flex-wrap gap-2">
                                    <button
                                        type="button"
                                        onClick={() => avatarInputRef.current?.click()}
                                        disabled={isAvatarUpdating}
                                        className="rounded-md border border-[var(--color-border)] px-2.5 py-1.5 text-[10px] font-semibold text-[var(--color-text-secondary)] transition-colors hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]"
                                    >
                                        {t("settings.persona.edit.avatar_choose")}
                                    </button>
                                    {editChar.avatar_path && (
                                        <button
                                            type="button"
                                            onClick={() => void handleRemoveAvatar()}
                                            disabled={isAvatarUpdating}
                                            className="rounded-md border border-[var(--color-error)]/40 px-2.5 py-1.5 text-[10px] font-semibold text-[var(--color-error)] transition-colors hover:bg-[var(--color-error)]/10"
                                        >
                                            {t("settings.persona.edit.avatar_remove")}
                                        </button>
                                    )}
                                </div>
                                <p className="mt-1 text-[10px] text-[var(--color-text-muted)]">
                                    {t("settings.persona.edit.avatar_desc")}
                                </p>
                            </div>
                        </div>
                    </div>

                    {/* Name */}
                    <div>
                        <label className="block text-[10px] font-heading font-semibold tracking-wider uppercase text-[var(--color-text-muted)] mb-1">
                            {t("settings.persona.edit.name")}
                        </label>
                        <input
                            type="text"
                            value={editChar.name}
                            onChange={e => handleFieldChange("name", e.target.value)}
                            onBlur={handleSaveEdit}
                            placeholder={t("settings.persona.edit.name_placeholder")}
                            className={inputClasses}
                        />
                    </div>

                    {/* User Nickname */}
                    <div>
                        <label className="block text-[10px] font-heading font-semibold tracking-wider uppercase text-[var(--color-text-muted)] mb-1">
                            {t("settings.persona.edit.nickname")}
                        </label>
                        <input
                            type="text"
                            value={editChar.user_nickname}
                            onChange={e => handleFieldChange("user_nickname", e.target.value)}
                            onBlur={handleSaveEdit}
                            placeholder={t("settings.persona.edit.nickname_placeholder")}
                            className={inputClasses}
                        />
                        <p className="text-[10px] text-[var(--color-text-muted)] mt-1 italic">
                            <Trans i18nKey="settings.persona.edit.nickname_desc" />
                        </p>
                    </div>

                    {/* Persona */}
                    <div>
                        <label className="block text-[10px] font-heading font-semibold tracking-wider uppercase text-[var(--color-text-muted)] mb-1">
                            {t("settings.persona.edit.persona")}
                        </label>
                        <textarea
                            value={editChar.persona}
                            onChange={e => handleFieldChange("persona", e.target.value)}
                            onBlur={handleSaveEdit}
                            placeholder={t("settings.persona.edit.persona_placeholder")}
                            rows={6}
                            className={clsx(inputClasses, "resize-y min-h-[100px]")}
                        />
                    </div>


                </div>
            )}
        </div>
    );
}

type AvatarPreviewProps = {
    readonly path: string | null;
    readonly resolveAvatarUrl?: (path: string) => string;
    readonly version?: number;
    readonly alt: string;
    readonly className: string;
};

function AvatarPreview(props: Readonly<AvatarPreviewProps>) {
    const [hasError, setHasError] = useState(false);

    useEffect(() => {
        setHasError(false);
    }, [props.path, props.version]);

    const canRenderImage = props.path !== null && props.resolveAvatarUrl !== undefined && !hasError;
    const resolvedUrl = canRenderImage ? props.resolveAvatarUrl?.(props.path ?? "") ?? null : null;
    const imageUrl = resolvedUrl === null || props.version === undefined
        ? resolvedUrl
        : `${resolvedUrl}${resolvedUrl.includes("?") ? "&" : "?"}v=${props.version}`;
    return (
        <span className={clsx("grid place-items-center overflow-hidden border border-[var(--color-border)] bg-black/20 text-[var(--color-text-secondary)]", props.className)}>
            {imageUrl !== null ? (
                <img
                    src={imageUrl}
                    alt={props.alt}
                    onError={() => setHasError(true)}
                    className="h-full w-full object-cover"
                />
            ) : (
                <UserCircle size={18} strokeWidth={1.5} aria-hidden="true" />
            )}
        </span>
    );
}
