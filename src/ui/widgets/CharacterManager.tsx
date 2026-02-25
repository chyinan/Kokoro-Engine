/**
 * CharacterManager — Persona tab replacement
 *
 * Full character management UI: list, create, edit, delete,
 * and import SillyTavern character cards (JSON / PNG).
 */
import { useState, useEffect, useCallback, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { Plus, Upload, Trash2, UserCircle, Check, X, User } from "lucide-react";
import { characterDb, type CharacterProfile } from "../../lib/db";
import { parseCharacterCard } from "../../lib/character-card-parser";
import { setPersona, setProactiveEnabled, getProactiveEnabled } from "../../lib/kokoro-bridge";
import { Languages, MessageCircle } from "lucide-react";
import { useTranslation, Trans } from "react-i18next";

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

function makeDefaultCharacter(): Omit<CharacterProfile, "id"> {
    return {
        name: "Kokoro",
        persona: DEFAULT_PERSONA,
        userNickname: "User",
        sourceFormat: "manual",
        createdAt: 0,
        updatedAt: 0,
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

function saveUserProfile(profile: UserProfile) {
    localStorage.setItem(USER_NAME_KEY, profile.name);
    localStorage.setItem(USER_PERSONA_KEY, profile.persona);
}

// ── Compose system prompt from a character ─────────

export function composeSystemPrompt(
    char: CharacterProfile | Omit<CharacterProfile, "id">,
    userProfile: UserProfile = loadUserProfile()
): string {
    const parts: string[] = [];
    parts.push(`Your name is ${char.name}.`);

    // Inject character persona with {{user}} replacement
    if (char.persona) {
        parts.push(char.persona.split("{{user}}").join(userProfile.name));
    }
    if (char.userNickname && char.userNickname !== "{{user}}") {
        parts.push(`Address the user as "${char.userNickname}".`);
    }

    // Inject user profile
    parts.push(`The user's name is ${userProfile.name}.`);
    if (userProfile.persona) {
        parts.push(`About the user: ${userProfile.persona}`);
    }

    return parts.join(" ");
}

// ── Storage key for active character ───────────────

const ACTIVE_CHAR_KEY = "kokoro_active_character_id";

// ── Props ──────────────────────────────────────────

interface CharacterManagerProps {
    /** Called whenever the active character changes so SettingsPanel can update its persona buffer */
    onPersonaChange: (prompt: string) => void;
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

export default function CharacterManager({ onPersonaChange, responseLanguage, onResponseLanguageChange, userLanguage, onUserLanguageChange }: CharacterManagerProps) {
    const { t } = useTranslation();
    const [characters, setCharacters] = useState<CharacterProfile[]>([]);
    const [activeId, setActiveId] = useState<number | null>(null);
    const [editChar, setEditChar] = useState<CharacterProfile | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [confirmDeleteId, setConfirmDeleteId] = useState<number | null>(null);
    const [importFeedback, setImportFeedback] = useState<string | null>(null);
    const [userProfile, setUserProfile] = useState<UserProfile>(loadUserProfile);
    const [proactiveEnabled, setProactiveEnabledState] = useState(true);

    // Stable ref for the callback to avoid re-triggering loadCharacters
    const onPersonaChangeRef = useRef(onPersonaChange);
    onPersonaChangeRef.current = onPersonaChange;

    // Fetch proactive setting from backend on mount
    useEffect(() => {
        getProactiveEnabled().then(setProactiveEnabledState).catch(() => {});
    }, []);

    // ── Load characters on mount ───────────────────

    const loadCharacters = useCallback(async () => {
        setIsLoading(true);
        try {
            const all = await characterDb.getAll();
            setCharacters(all);

            // Restore active selection
            const savedId = localStorage.getItem(ACTIVE_CHAR_KEY);
            if (savedId && all.find(c => c.id === Number(savedId))) {
                const id = Number(savedId);
                setActiveId(id);
                const char = all.find(c => c.id === id)!;
                setEditChar(char);
                const prompt = composeSystemPrompt(char);
                onPersonaChangeRef.current(prompt);
                setPersona(prompt).catch(e => console.error("[CharacterManager] Failed to set persona:", e));
            } else if (all.length > 0) {
                setActiveId(all[0].id!);
                setEditChar(all[0]);
                const prompt = composeSystemPrompt(all[0]);
                onPersonaChangeRef.current(prompt);
                setPersona(prompt).catch(e => console.error("[CharacterManager] Failed to set persona:", e));
            } else {
                // No characters yet — insert default
                const defaultChar = makeDefaultCharacter();
                const newId = await characterDb.add(defaultChar);
                const created = { ...defaultChar, id: newId };
                setCharacters([created]);
                setActiveId(newId);
                setEditChar(created);
                const prompt = composeSystemPrompt(created);
                onPersonaChangeRef.current(prompt);
                setPersona(prompt).catch(e => console.error("[CharacterManager] Failed to set persona:", e));
                localStorage.setItem(ACTIVE_CHAR_KEY, String(newId));
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

    // ── Select a character ─────────────────────────

    const selectCharacter = (char: CharacterProfile) => {
        setActiveId(char.id!);
        setEditChar({ ...char });
        setConfirmDeleteId(null);
        localStorage.setItem(ACTIVE_CHAR_KEY, String(char.id));
        const prompt = composeSystemPrompt(char);
        onPersonaChangeRef.current(prompt);
        // Immediately push to backend so LLM uses the new persona
        setPersona(prompt).catch(e => console.error("[CharacterManager] Failed to set persona:", e));
    };

    // ── Create new character ───────────────────────

    const handleCreate = async () => {
        const now = Date.now();
        const newChar: Omit<CharacterProfile, "id"> = {
            name: "New Character",
            persona: "",
            userNickname: "User",
            sourceFormat: "manual",
            createdAt: now,
            updatedAt: now,
        };
        try {
            const newId = await characterDb.add(newChar);
            const created = { ...newChar, id: newId };
            setCharacters(prev => [...prev, created]);
            selectCharacter(created);
        } catch (err) {
            console.error("[CharacterManager] Failed to create character:", err);
        }
    };

    // ── Update current character ───────────────────

    const handleFieldChange = (field: keyof CharacterProfile, value: string) => {
        if (!editChar) return;
        setEditChar(prev => prev ? { ...prev, [field]: value } : null);
    };

    const handleSaveEdit = async () => {
        if (!editChar || !editChar.id) return;
        try {
            await characterDb.update(editChar);
            setCharacters(prev => prev.map(c => c.id === editChar.id ? { ...editChar } : c));
            const prompt = composeSystemPrompt(editChar);
            onPersonaChangeRef.current(prompt);
            // Push updated persona to backend immediately
            setPersona(prompt).catch(e => console.error("[CharacterManager] Failed to set persona:", e));
        } catch (err) {
            console.error("[CharacterManager] Failed to update character:", err);
        }
    };

    // ── Delete character ───────────────────────────

    const handleDelete = async (charId: number) => {
        try {
            await characterDb.remove(charId);
            const remaining = characters.filter(c => c.id !== charId);
            setCharacters(remaining);
            setConfirmDeleteId(null);

            // If we deleted the active/editing character, switch
            if (activeId === charId || editChar?.id === charId) {
                if (remaining.length > 0) {
                    selectCharacter(remaining[0]);
                } else {
                    // Re-create default
                    const defaultChar = makeDefaultCharacter();
                    const newId = await characterDb.add(defaultChar);
                    const created = { ...defaultChar, id: newId };
                    setCharacters([created]);
                    selectCharacter(created);
                }
            }
        } catch (err) {
            console.error("[CharacterManager] Failed to delete character:", err);
        }
    };

    // ── Import character card ──────────────────────

    const handleImport = async () => {
        // Create a hidden file input and click it
        const input = document.createElement("input");
        input.type = "file";
        input.accept = ".json,.png";
        input.onchange = async (e) => {
            const file = (e.target as HTMLInputElement).files?.[0];
            if (!file) return;

            try {
                const profile = await parseCharacterCard(file);
                const now = Date.now();
                const newId = await characterDb.add({
                    ...profile,
                    createdAt: now,
                    updatedAt: now,
                });
                const created: CharacterProfile = { ...profile, id: newId, createdAt: now, updatedAt: now };
                setCharacters(prev => [...prev, created]);
                selectCharacter(created);
                setImportFeedback(t("settings.persona.status.imported", { name: profile.name }));
                setTimeout(() => setImportFeedback(null), 3000);
            } catch (err) {
                console.error("[CharacterManager] Import failed:", err);
                setImportFeedback(t("settings.persona.status.import_failed", { error: err instanceof Error ? err.message : "Unknown error" }));
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
        // Re-compose the active character's prompt with updated user info
        if (editChar) {
            onPersonaChangeRef.current(composeSystemPrompt(editChar, userProfile));
            // Push updated persona to backend immediately
            setPersona(composeSystemPrompt(editChar, userProfile)).catch(e => console.error("[CharacterManager] Failed to set persona:", e));
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
                <select
                    value={["auto", "日本語", "English", "中文", "한국어"].includes(responseLanguage || "auto") ? (responseLanguage || "auto") : "__custom__"}
                    onChange={e => {
                        const v = e.target.value;
                        if (v === "auto") onResponseLanguageChange("");
                        else if (v === "__custom__") onResponseLanguageChange("");
                        else onResponseLanguageChange(v);
                    }}
                    className={inputClasses}
                >
                    <option value="auto">{t("settings.persona.response_lang.auto")}</option>
                    <option value="日本語">日本語 (Japanese)</option>
                    <option value="English">English</option>
                    <option value="中文">中文 (Chinese)</option>
                    <option value="한국어">한국어 (Korean)</option>
                    <option value="__custom__">{t("settings.persona.response_lang.custom")}</option>
                </select>
                {/* Show custom input when language is not in presets */}
                {responseLanguage && !["日本語", "English", "中文", "한국어"].includes(responseLanguage) && (
                    <input
                        type="text"
                        value={responseLanguage}
                        onChange={e => onResponseLanguageChange(e.target.value)}
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
                <select
                    value={["auto", "中文", "English", "日本語", "한국어"].includes(userLanguage || "auto") ? (userLanguage || "auto") : "__custom__"}
                    onChange={e => {
                        const v = e.target.value;
                        if (v === "auto") onUserLanguageChange("");
                        else if (v === "__custom__") onUserLanguageChange("");
                        else onUserLanguageChange(v);
                    }}
                    className={inputClasses}
                >
                    <option value="auto">{t("settings.persona.user_lang.auto")}</option>
                    <option value="中文">中文 (Chinese)</option>
                    <option value="English">English</option>
                    <option value="日本語">日本語 (Japanese)</option>
                    <option value="한국어">한국어 (Korean)</option>
                    <option value="__custom__">{t("settings.persona.response_lang.custom")}</option>
                </select>
                {userLanguage && !["中文", "English", "日本語", "한국어"].includes(userLanguage) && (
                    <input
                        type="text"
                        value={userLanguage}
                        onChange={e => onUserLanguageChange(e.target.value)}
                        placeholder={t("settings.persona.response_lang.placeholder")}
                        className={clsx(inputClasses, "mt-2")}
                    />
                )}
            </div>

            {/* ── Proactive Messages (Idle Auto-Talk) ── */}
            <div>
                <label className={labelClasses}>
                    <MessageCircle size={12} strokeWidth={2} className="inline-block mr-1.5 -mt-0.5" />
                    {t("settings.persona.proactive.label")}
                </label>
                <p className="text-[10px] text-[var(--color-text-muted)] mb-3 -mt-1">
                    {t("settings.persona.proactive.desc")}
                </p>
                <button
                    onClick={() => {
                        const next = !proactiveEnabled;
                        setProactiveEnabledState(next);
                        setProactiveEnabled(next).catch(e => console.error("[CharacterManager] Failed to set proactive:", e));
                    }}
                    className={clsx(
                        "relative inline-flex h-6 w-11 items-center rounded-full transition-colors",
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
                            importFeedback.startsWith("Import failed")
                                ? "bg-[var(--color-error)]/10 text-[var(--color-error)]"
                                : "bg-[var(--color-accent-subtle)] text-[var(--color-accent)]"
                        )}
                    >
                        {importFeedback}
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
                                            onClick={() => handleDelete(char.id!)}
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
                            <UserCircle size={16} strokeWidth={1.5} className="shrink-0 opacity-60" />
                            <div className="flex-1 min-w-0">
                                <span className="text-sm font-heading font-semibold tracking-wide truncate block">
                                    {char.name}
                                </span>
                                {char.sourceFormat && char.sourceFormat !== "manual" && (
                                    <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wider">
                                        {char.sourceFormat}
                                    </span>
                                )}
                            </div>
                            {activeId === char.id && (
                                <Check size={14} strokeWidth={2} className="text-[var(--color-accent)] shrink-0" />
                            )}
                            {/* Delete icon — visible on hover */}
                            <motion.div
                                whileHover={{ scale: 1.15 }}
                                whileTap={{ scale: 0.9 }}
                                onClick={(e) => {
                                    e.stopPropagation();
                                    setConfirmDeleteId(char.id!);
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
                            value={editChar.userNickname}
                            onChange={e => handleFieldChange("userNickname", e.target.value)}
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
