import { useState, useEffect } from "react";
import { clsx } from "clsx";
import { useTranslation } from "react-i18next";
import { FolderOpen, RefreshCw, Trash2, Check, Download, Pencil } from "lucide-react";
import { motion, AnimatePresence } from "framer-motion";
import { open, save } from "@tauri-apps/plugin-dialog";
import { labelClasses } from "../../styles/settings-primitives";
import { Select } from "@/components/ui/select";
import {
    importLive2dZip,
    importLive2dFolder,
    exportLive2dModel,
    listLive2dModels,
    deleteLive2dModel,
    renameLive2dModel,
    getLive2dModelProfile,
    saveLive2dModelProfile,
    playCue,
    BUILTIN_LIVE2D_MODEL_PATH,
} from "../../../lib/kokoro-bridge";
import type { Live2dCueBinding, Live2dModelInfo, Live2dModelProfile } from "../../../lib/kokoro-bridge";
import type { Live2DDisplayMode } from "../../../features/live2d/Live2DViewer";

const INTERACTION_GESTURES = [
    { value: "tap" },
    { value: "long_press" },
    { value: "rapid_tap" },
    { value: "*" },
] as const;

const INTERACTION_AREAS = [
    { value: "eyes" },
    { value: "lips" },
    { value: "face" },
    { value: "head" },
    { value: "hair" },
    { value: "hand" },
    { value: "arm" },
    { value: "chest" },
    { value: "body" },
    { value: "leg" },
    { value: "skirt" },
    { value: "accessory" },
    { value: "*" },
] as const;

const PRESET_INTERACTION_AREA_VALUES = INTERACTION_AREAS.map((area) => area.value);

export function sanitizeSemanticCueMap(input: Record<string, string>): Record<string, string> {
    return Object.fromEntries(
        Object.entries(input).filter(([key]) => key.startsWith("interaction:")),
    );
}

const buildInteractionSemanticKey = (gesture: string, area: string) =>
    `interaction:${gesture.toLowerCase()}_${area.toLowerCase()}`;

const parseInteractionSemanticKey = (key: string) => {
    const suffix = key.startsWith("interaction:") ? key.slice("interaction:".length) : key;
    const lastUnderscore = suffix.lastIndexOf("_");
    if (lastUnderscore === -1) return { gesture: suffix || "tap", area: "face" };
    return { gesture: suffix.slice(0, lastUnderscore) || "tap", area: suffix.slice(lastUnderscore + 1) || "face" };
};

export interface ModelTabProps {
    displayMode: Live2DDisplayMode;
    onDisplayModeChange: (mode: Live2DDisplayMode) => void;
    customModelPath: string | null;
    onCustomModelPathChange: (path: string | null) => void;
    gazeTracking?: boolean;
    onGazeTrackingChange?: (enabled: boolean) => void;
    renderFps: number;
    onRenderFpsChange: (fps: number) => void;
}

export default function ModelTab({
    displayMode, onDisplayModeChange,
    customModelPath, onCustomModelPathChange,
    gazeTracking = true, onGazeTrackingChange,
    renderFps, onRenderFpsChange,
}: ModelTabProps) {
    const { t } = useTranslation();
    const interactionGestureLabel = (value: string) => t(`settings.model.mapping.gestures.${value === "*" ? "any" : value}`);
    const interactionAreaLabel = (value: string) => {
        const key = value === "*" ? "any" : value;
        const translated = t(`settings.model.mapping.areas.${key}`);
        return translated === `settings.model.mapping.areas.${key}` ? value : translated;
    };
    const [isImporting, setIsImporting] = useState(false);
    const [isExporting, setIsExporting] = useState(false);
    const [models, setModels] = useState<Live2dModelInfo[]>([]);
    const [isLoadingModels, setIsLoadingModels] = useState(false);
    const [showBuiltin, setShowBuiltin] = useState(
        () => localStorage.getItem("modelTab.showBuiltin") !== "false"
    );
    const [modelProfile, setModelProfile] = useState<Live2dModelProfile | null>(null);
    const [isProfileLoading, setIsProfileLoading] = useState(false);
    const [editingCueKey, setEditingCueKey] = useState<string | null>(null);
    const [draftCue, setDraftCue] = useState("");
    const [draftExpression, setDraftExpression] = useState("");
    const [draftMotionGroup, setDraftMotionGroup] = useState("");
    const [excludedPromptCue, setExcludedPromptCue] = useState("");
    const [interactionGesture, setInteractionGesture] = useState<(typeof INTERACTION_GESTURES)[number]["value"]>("tap");
    const [interactionArea, setInteractionArea] = useState("face");
    const [interactionCue, setInteractionCue] = useState("");
    const [editingInteractionKey, setEditingInteractionKey] = useState<string | null>(null);
    const effectiveModelPath = customModelPath ?? BUILTIN_LIVE2D_MODEL_PATH;

    // Fetch available models on mount
    useEffect(() => {
        fetchModels();
    }, []);

    useEffect(() => {
        setIsProfileLoading(true);
        getLive2dModelProfile(effectiveModelPath)
            .then(async (profile) => {
                const originalSemanticMap = profile.semantic_cue_map ?? {};
                const sanitizedSemanticMap = sanitizeSemanticCueMap(originalSemanticMap);

                if (Object.keys(sanitizedSemanticMap).length !== Object.keys(originalSemanticMap).length) {
                    const saved = await saveLive2dModelProfile({
                        ...profile,
                        semantic_cue_map: sanitizedSemanticMap,
                    });
                    setModelProfile(saved);
                    return;
                }

                setModelProfile(profile);
            })
            .catch((error) => {
                console.error("Failed to load Live2D profile:", error);
                setModelProfile(null);
            })
            .finally(() => {
                setIsProfileLoading(false);
            });
    }, [effectiveModelPath]);

    const fetchModels = async () => {
        setIsLoadingModels(true);
        try {
            const list = await listLive2dModels();
            setModels(list);
        } catch (e) {
            console.error("Failed to list Live2D models:", e);
        } finally {
            setIsLoadingModels(false);
        }
    };

    const handleImportModel = async () => {
        try {
            const selected = await open({
                multiple: false,
                filters: [
                    { name: 'Live2D Package (zip)', extensions: ['zip'] },
                    { name: 'Live2D Model', extensions: ['model3.json'] },
                ]
            });

            if (selected && typeof selected === 'string') {
                if (selected.toLowerCase().endsWith('.zip')) {
                    setIsImporting(true);
                    try {
                        const modelPath = await importLive2dZip(selected);
                        onCustomModelPathChange(modelPath);
                        // Refresh model list after import
                        await fetchModels();
                    } catch (e) {
                        console.error("Failed to import Live2D zip:", e);
                    } finally {
                        setIsImporting(false);
                    }
                } else {
                    // .model3.json selected — copy the folder into app data
                    setIsImporting(true);
                    try {
                        const modelPath = await importLive2dFolder(selected);
                        onCustomModelPathChange(modelPath);
                        await fetchModels();
                    } catch (e) {
                        console.error("Failed to import Live2D folder:", e);
                    } finally {
                        setIsImporting(false);
                    }
                }
            }
        } catch (err) {
            console.error("Failed to open file dialog:", err);
        }
    };

    const handleDeleteModel = async (modelName: string, modelPath: string) => {
        try {
            await deleteLive2dModel(modelName);
            // If we deleted the currently active model, switch to default
            if (customModelPath === modelPath) {
                onCustomModelPathChange(null);
            }
            // Refresh list
            await fetchModels();
        } catch (e) {
            console.error("Failed to delete model:", e);
        }
    };

    const handleRenameModel = async (model: Live2dModelInfo) => {
        const nextName = window.prompt(t("settings.model.rename.prompt"), model.name);
        if (!nextName || nextName.trim() === model.name) return;
        try {
            const nextPath = await renameLive2dModel(model.path, nextName);
            if (customModelPath === model.path) {
                onCustomModelPathChange(nextPath);
            }
            await fetchModels();
        } catch (e) {
            console.error("Failed to rename model:", e);
        }
    };

    const selectedImportedModel = models.find((model) => model.path === customModelPath) ?? null;

    const handleExportModel = async () => {
        if (!selectedImportedModel) return;
        try {
            const filePath = await save({
                defaultPath: `${selectedImportedModel.name}.zip`,
                filters: [{ name: "Live2D Package", extensions: ["zip"] }],
            });
            if (!filePath) return;

            setIsExporting(true);
            await exportLive2dModel(selectedImportedModel.path, filePath);
        } catch (error) {
            console.error("Failed to export Live2D model:", error);
        } finally {
            setIsExporting(false);
        }
    };

    const isSelected = (path: string | null) => {
        return customModelPath === path;
    };

    const resetCueDraft = () => {
        setEditingCueKey(null);
        setDraftCue("");
        setDraftExpression("");
        setDraftMotionGroup("");
    };

    const resetInteractionDraft = () => {
        setEditingInteractionKey(null);
        setInteractionGesture("tap");
        setInteractionArea("face");
        setInteractionCue("");
    };


    const persistCueMap = async (cueMap: Record<string, Live2dCueBinding>) => {
        if (!modelProfile) return;
        try {
            const saved = await saveLive2dModelProfile({
                ...modelProfile,
                cue_map: cueMap,
            });
            setModelProfile(saved);
        } catch (error) {
            console.error("Failed to save cue map:", error);
        }
    };

    const handleSaveCue = async () => {
        const cue = draftCue.trim();
        if (!cue || !modelProfile) return;
        if (!draftExpression && !draftMotionGroup) return;

        const nextCueMap = {
            ...modelProfile.cue_map,
            [cue]: {
                expression: draftExpression || null,
                motion_group: draftMotionGroup || null,
                exclude_from_prompt: false,
            },
        };

        if (editingCueKey && editingCueKey !== cue) {
            delete nextCueMap[editingCueKey];
        }

        await persistCueMap(nextCueMap);
        resetCueDraft();
    };

    const handleEditCue = (cue: string, binding: Live2dCueBinding) => {
        setEditingCueKey(cue);
        setDraftCue(cue);
        setDraftExpression(binding.expression ?? "");
        setDraftMotionGroup(binding.motion_group ?? "");
    };

    const handleDeleteCue = async (cue: string) => {
        if (!modelProfile) return;
        const nextCueMap = { ...modelProfile.cue_map };
        delete nextCueMap[cue];
        await persistCueMap(nextCueMap);
        if (draftCue.trim() === cue) {
            resetCueDraft();
        }
    };

    const handleToggleCuePrompt = async (cue: string, excluded: boolean) => {
        if (!modelProfile) return;
        const binding = modelProfile.cue_map[cue];
        if (!binding) return;
        await persistCueMap({
            ...modelProfile.cue_map,
            [cue]: {
                ...binding,
                exclude_from_prompt: excluded,
            },
        });
    };

    const handlePreviewCue = async (cue: string) => {
        const trimmed = cue.trim();
        if (!trimmed) return;
        try {
            await playCue(trimmed);
        } catch (error) {
            console.error("Failed to preview cue:", error);
        }
    };

    const handleSaveInteractionMapping = async () => {
        if (!modelProfile) return;
        const cue = interactionCue.trim();
        if (!cue) return;

        const key = buildInteractionSemanticKey(interactionGesture, interactionArea);
        const nextMap = { ...modelProfile.semantic_cue_map, [key]: cue };
        if (editingInteractionKey && editingInteractionKey !== key) {
            delete nextMap[editingInteractionKey];
        }
        await persistSemanticMap(nextMap);
        resetInteractionDraft();
    };

    const handleEditInteractionMapping = (key: string, cue: string) => {
        setEditingInteractionKey(key);
        const { gesture, area } = parseInteractionSemanticKey(key);
        setInteractionGesture((INTERACTION_GESTURES.some((item) => item.value === gesture) ? gesture : "tap") as (typeof INTERACTION_GESTURES)[number]["value"]);
        setInteractionArea(area);
        setInteractionCue(cue);
    };

    const handleDeleteInteractionMapping = async (key: string) => {
        if (!modelProfile) return;
        const nextMap = { ...modelProfile.semantic_cue_map };
        delete nextMap[key];
        await persistSemanticMap(nextMap);
    };

    const persistSemanticMap = async (semanticMap: Record<string, string>) => {
        if (!modelProfile) return;
        try {
            const saved = await saveLive2dModelProfile({
                ...modelProfile,
                semantic_cue_map: semanticMap,
            });
            setModelProfile(saved);
        } catch (error) {
            console.error("Failed to save semantic cue map:", error);
        }
    };


    const displayModes = [
        { mode: "full" as Live2DDisplayMode, label: t("settings.model.display_mode.full"), desc: t("settings.model.display_mode.full_desc") },
        { mode: "upper-thigh" as Live2DDisplayMode, label: t("settings.model.display_mode.upper_thigh"), desc: t("settings.model.display_mode.upper_thigh_desc") },
        { mode: "upper" as Live2DDisplayMode, label: t("settings.model.display_mode.upper"), desc: t("settings.model.display_mode.upper_desc") },
    ];

    const renderFpsPreset = renderFps === 30 || renderFps === 60 || renderFps === 0
        ? String(renderFps)
        : "custom";

    const handleRenderFpsPresetChange = (value: string) => {
        if (value === "custom") {
            onRenderFpsChange(renderFps > 0 && renderFps !== 30 && renderFps !== 60 ? renderFps : 45);
            return;
        }

        onRenderFpsChange(Number(value));
    };

    const handleCustomRenderFpsChange = (value: string) => {
        const parsed = Number.parseInt(value, 10);
        onRenderFpsChange(Number.isFinite(parsed) && parsed > 0 ? parsed : 1);
    };

    const interactionEntries = modelProfile
        ? Object.entries(modelProfile.semantic_cue_map).filter(([key]) => key.startsWith("interaction:"))
        : [];
    const availableCueNames = modelProfile
        ? Object.keys(modelProfile.cue_map).sort((a, b) => a.localeCompare(b))
        : [];
    const interactionAreaOptions = Array.from(new Set([
        ...PRESET_INTERACTION_AREA_VALUES,
        ...(modelProfile?.available_hit_areas ?? []).map((area) => area.toLowerCase()),
        interactionArea,
        ...interactionEntries.map(([key]) => parseInteractionSemanticKey(key).area.toLowerCase()),
    ]));
    const excludedPromptCueNames = modelProfile
        ? Object.entries(modelProfile.cue_map)
            .filter(([, binding]) => binding.exclude_from_prompt)
            .map(([cue]) => cue)
            .sort((a, b) => a.localeCompare(b))
        : [];
    const includablePromptCueNames = modelProfile
        ? availableCueNames.filter((cue) => !modelProfile.cue_map[cue]?.exclude_from_prompt)
        : [];

    return (
        <div className="space-y-5">
            <div>
                <label className={labelClasses}>{t("settings.model.display_mode.label")}</label>
                <p className="text-xs text-[var(--color-text-muted)] mb-3">
                    {t("settings.model.display_mode.desc")}
                </p>
                <div className="grid grid-cols-3 gap-3">
                    {displayModes.map(({ mode, label, desc }) => (
                        <motion.button
                            key={mode}
                            whileHover={{ scale: 1.02 }}
                            whileTap={{ scale: 0.97 }}
                            onClick={() => onDisplayModeChange(mode)}
                            className={clsx(
                                "flex flex-col items-center gap-2 p-4 rounded-xl border text-center transition-colors",
                                displayMode === mode
                                    ? "bg-[var(--color-accent-subtle)] border-[var(--color-accent)]/40 text-[var(--color-accent)]"
                                    : "border-[var(--color-border)] text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] hover:border-[var(--color-text-muted)]"
                            )}
                        >
                            {/* Visual indicator */}
                            <div className="h-14 flex items-center justify-center">
                                <div className={clsx(
                                    "w-8 rounded-sm border-2 transition-colors",
                                    displayMode === mode ? "border-[var(--color-accent)]" : "border-[var(--color-border)]",
                                    mode === "full" ? "h-14" : mode === "upper-thigh" ? "h-10" : "h-7"
                                )} />
                            </div>
                            <span className="text-sm font-heading font-semibold tracking-wider uppercase">{label}</span>
                            <span className="text-[10px] text-[var(--color-text-muted)]">{desc}</span>
                        </motion.button>
                    ))}
                </div>
            </div>

            {/* Gaze Tracking Toggle */}
            <div className="flex items-center justify-between py-3 px-1">
                <div>
                    <label className={labelClasses}>{t("settings.model.gaze_tracking.label")}</label>
                    <p className="text-xs text-[var(--color-text-muted)] mt-0.5">
                        {t("settings.model.gaze_tracking.desc")}
                    </p>
                </div>
                <button
                    onClick={() => onGazeTrackingChange?.(!gazeTracking)}
                    className={clsx(
                        "relative w-10 h-5 rounded-full transition-colors flex-shrink-0 ml-4",
                        gazeTracking
                            ? "bg-[var(--color-accent)]"
                            : "bg-[var(--color-border)]"
                    )}
                    role="switch"
                    aria-checked={gazeTracking}
                    aria-label={t("settings.model.gaze_tracking.label")}
                >
                    <span className={clsx(
                        "absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform",
                        gazeTracking && "translate-x-5"
                    )} />
                </button>
            </div>

            <div>
                <label className={labelClasses}>{t("settings.model.render_fps.label")}</label>
                <div className="mt-3 flex items-center gap-3">
                    <Select
                        value={renderFpsPreset}
                        onChange={handleRenderFpsPresetChange}
                        options={[
                            { value: "30", label: t("settings.model.render_fps.options.fps_30") },
                            { value: "60", label: t("settings.model.render_fps.options.fps_60") },
                            { value: "0", label: t("settings.model.render_fps.options.unlimited") },
                            { value: "custom", label: t("settings.model.render_fps.options.custom") },
                        ]}
                    />
                    {renderFpsPreset === "custom" && (
                        <input
                            type="number"
                            min={1}
                            step={1}
                            value={renderFps}
                            onChange={(e) => handleCustomRenderFpsChange(e.target.value)}
                            className="w-28 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-surface-soft)] px-3 py-2 text-sm text-[var(--color-text-primary)] outline-none [&::-webkit-outer-spin-button]:hidden [&::-webkit-inner-spin-button]:hidden [&]:[-moz-appearance:textfield]"
                        />
                    )}
                </div>
            </div>

            {/* Model List Section */}
            <div>
                <div className="flex items-center justify-between mb-2">
                    <label className={labelClasses}>{t("settings.model.list.label")}</label>
                    <motion.button
                        whileHover={{ scale: 1.1 }}
                        whileTap={{ scale: 0.9 }}
                        onClick={fetchModels}
                        disabled={isLoadingModels}
                        className="p-1.5 rounded-md text-[var(--color-text-muted)] hover:text-[var(--color-accent)] hover:bg-[var(--color-accent-subtle)] transition-colors"
                        title="Refresh model list"
                    >
                        <RefreshCw size={14} strokeWidth={1.5} className={isLoadingModels ? "animate-spin" : ""} />
                    </motion.button>
                </div>
                <div className="space-y-2 max-h-[220px] overflow-y-auto scrollable pr-1">
                    {/* Default Model (hideable) */}
                    {showBuiltin && <div
                        className={clsx(
                            "flex items-center gap-3 px-3 py-2.5 rounded-lg border transition-all",
                            isSelected(null)
                                ? "bg-[var(--color-accent-subtle)] border-[var(--color-accent)]/40"
                                : "bg-black/20 border-[var(--color-border)] hover:border-[var(--color-text-muted)]"
                        )}
                    >
                        <motion.button
                            whileHover={{ scale: 1.01 }}
                            whileTap={{ scale: 0.99 }}
                            onClick={() => onCustomModelPathChange(null)}
                            className="flex items-center gap-3 flex-1 min-w-0 text-left"
                        >
                            {/* Selection indicator */}
                            <div className={clsx(
                                "w-5 h-5 rounded-full border-2 flex items-center justify-center flex-shrink-0 transition-colors",
                                isSelected(null)
                                    ? "border-[var(--color-accent)] bg-[var(--color-accent)]"
                                    : "border-[var(--color-border)]"
                            )}>
                                {isSelected(null) && <Check size={12} strokeWidth={3} className="text-black" />}
                            </div>
                            <div className="flex-1 min-w-0">
                                <span className={clsx(
                                    "text-sm font-medium truncate block",
                                    isSelected(null) ? "text-[var(--color-accent)]" : "text-[var(--color-text-primary)]"
                                )}>
                                    {t("settings.model.list.default")}
                                </span>
                                <span className="text-[10px] text-[var(--color-text-muted)]">{t("settings.model.list.builtin")}</span>
                            </div>
                        </motion.button>
                        {/* Delete button — hides the builtin card (persisted) */}
                        <motion.button
                            whileHover={{ scale: 1.1 }}
                            whileTap={{ scale: 0.9 }}
                            onClick={() => {
                                localStorage.setItem("modelTab.showBuiltin", "false");
                                setShowBuiltin(false);
                                if (isSelected(null)) onCustomModelPathChange(models[0]?.path ?? null);
                            }}
                            className="p-1.5 rounded-md text-[var(--color-text-muted)] hover:text-[var(--color-error)] hover:bg-[var(--color-error)]/10 transition-colors flex-shrink-0"
                            title={t("common.actions.delete")}
                        >
                            <Trash2 size={14} strokeWidth={1.5} />
                        </motion.button>
                    </div>}

                    {/* Imported Models */}
                    <AnimatePresence>
                        {models.map((model) => (
                            <motion.div
                                key={model.name}
                                initial={{ opacity: 0, y: -10 }}
                                animate={{ opacity: 1, y: 0 }}
                                exit={{ opacity: 0, x: -20 }}
                                className={clsx(
                                    "flex items-center gap-3 px-3 py-2.5 rounded-lg border transition-all",
                                    isSelected(model.path)
                                        ? "bg-[var(--color-accent-subtle)] border-[var(--color-accent)]/40"
                                        : "bg-black/20 border-[var(--color-border)] hover:border-[var(--color-text-muted)]"
                                )}
                            >
                                <motion.button
                                    whileHover={{ scale: 1.01 }}
                                    whileTap={{ scale: 0.99 }}
                                    onClick={() => onCustomModelPathChange(model.path)}
                                    className="flex items-center gap-3 flex-1 min-w-0 text-left"
                                >
                                    {/* Selection indicator */}
                                    <div className={clsx(
                                        "w-5 h-5 rounded-full border-2 flex items-center justify-center flex-shrink-0 transition-colors",
                                        isSelected(model.path)
                                            ? "border-[var(--color-accent)] bg-[var(--color-accent)]"
                                            : "border-[var(--color-border)]"
                                    )}>
                                        {isSelected(model.path) && <Check size={12} strokeWidth={3} className="text-black" />}
                                    </div>
                                    <div className="flex-1 min-w-0">
                                        <span className={clsx(
                                            "text-sm font-medium truncate block",
                                            isSelected(model.path) ? "text-[var(--color-accent)]" : "text-[var(--color-text-primary)]"
                                        )}>
                                            {model.name}
                                        </span>
                                        <span className="text-[10px] text-[var(--color-text-muted)] truncate block">{model.path}</span>
                                    </div>
                                </motion.button>
                                {/* Delete button */}
                                <motion.button
                                    whileHover={{ scale: 1.1 }}
                                    whileTap={{ scale: 0.9 }}
                                    onClick={() => handleRenameModel(model)}
                                    className="p-1.5 rounded-md text-[var(--color-text-muted)] hover:text-[var(--color-accent)] hover:bg-[var(--color-accent)]/10 transition-colors flex-shrink-0"
                                    title={t("common.actions.rename")}
                                >
                                    <Pencil size={14} strokeWidth={1.5} />
                                </motion.button>
                                <motion.button
                                    whileHover={{ scale: 1.1 }}
                                    whileTap={{ scale: 0.9 }}
                                    onClick={() => handleDeleteModel(model.name, model.path)}
                                    className="p-1.5 rounded-md text-[var(--color-text-muted)] hover:text-[var(--color-error)] hover:bg-[var(--color-error)]/10 transition-colors flex-shrink-0"
                                    title={`Delete ${model.name}`}
                                >
                                    <Trash2 size={14} strokeWidth={1.5} />
                                </motion.button>
                            </motion.div>
                        ))}
                    </AnimatePresence>

                    {models.length === 0 && !isLoadingModels && (
                        <p className="text-xs text-[var(--color-text-muted)] text-center py-2 italic">
                            {t("settings.model.list.empty")}
                        </p>
                    )}
                </div>

                {/* Import button */}
                <motion.button
                    whileHover={{ scale: 1.02 }}
                    whileTap={{ scale: 0.98 }}
                    onClick={handleImportModel}
                    disabled={isImporting}
                    className={clsx(
                        "w-full flex items-center justify-center gap-2 px-4 py-2.5 rounded-lg text-sm font-heading font-semibold tracking-wider uppercase mt-3",
                        "bg-[var(--color-bg-elevated)] border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                        "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors",
                        isImporting && "opacity-60 cursor-not-allowed"
                    )}
                >
                    {isImporting ? (
                        <RefreshCw size={16} strokeWidth={1.5} className="animate-spin" />
                    ) : (
                        <FolderOpen size={16} strokeWidth={1.5} />
                    )}
                    {isImporting ? t("settings.model.import.loading") : t("settings.model.import.button")}
                </motion.button>

                <motion.button
                    whileHover={{ scale: 1.02 }}
                    whileTap={{ scale: 0.98 }}
                    onClick={handleExportModel}
                    disabled={!selectedImportedModel || isExporting}
                    className={clsx(
                        "w-full flex items-center justify-center gap-2 px-4 py-2.5 rounded-lg text-sm font-heading font-semibold tracking-wider uppercase mt-2",
                        "bg-[var(--color-bg-elevated)] border border-[var(--color-border)] text-[var(--color-text-secondary)]",
                        "hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors",
                        (!selectedImportedModel || isExporting) && "opacity-60 cursor-not-allowed"
                    )}
                >
                    {isExporting ? (
                        <RefreshCw size={16} strokeWidth={1.5} className="animate-spin" />
                    ) : (
                        <Download size={16} strokeWidth={1.5} />
                    )}
                    {isExporting ? t("settings.model.export.loading") : t("settings.model.export.button")}
                </motion.button>
            </div>

            <div className="space-y-3 rounded-xl border border-[var(--color-border)] bg-black/10 p-4">
                <div>
                    <label className={labelClasses}>{t("settings.model.mapping.cues.title")}</label>
                    <p className="text-xs text-[var(--color-text-muted)] mt-1">
                        {t("settings.model.mapping.cues.desc")}
                    </p>
                </div>

                {isProfileLoading && (
                    <p className="text-sm text-[var(--color-text-muted)]">{t("settings.model.mapping.loading_profile")}</p>
                )}

                {!isProfileLoading && !modelProfile && (
                    <p className="text-sm text-[var(--color-text-muted)]">
                        {t("settings.model.mapping.select_model")}
                    </p>
                )}

                {modelProfile && (
                    <>
                        <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
                            <input
                                value={draftCue}
                                onChange={(e) => setDraftCue(e.target.value)}
                                placeholder={t("settings.model.mapping.placeholders.cue_name")}
                                className="rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-surface-soft)] px-3 py-2 text-sm text-[var(--color-text-primary)] outline-none"
                            />
                            <Select
                                value={draftExpression}
                                onChange={setDraftExpression}
                                options={[
                                    { value: "", label: t("settings.model.mapping.options.no_expression") },
                                    ...modelProfile.available_expressions.map((expression) => ({ value: expression, label: expression })),
                                ]}
                            />
                            <Select
                                value={draftMotionGroup}
                                onChange={setDraftMotionGroup}
                                options={[
                                    { value: "", label: t("settings.model.mapping.options.no_motion_group") },
                                    ...Object.entries(modelProfile.available_motion_groups).map(([group, count]) => ({ value: group, label: `${group} (${count})` })),
                                ]}
                            />
                        </div>

                        <div className="flex items-center gap-2">
                            <button
                                onClick={handleSaveCue}
                                disabled={!draftCue.trim() || (!draftExpression && !draftMotionGroup)}
                                className="rounded-lg bg-[var(--color-accent)] px-3 py-2 text-sm font-medium text-white disabled:cursor-not-allowed disabled:opacity-50"
                            >
                                {t("settings.model.mapping.actions.save_cue")}
                            </button>
                            <button
                                onClick={resetCueDraft}
                                className="rounded-lg border border-[var(--color-border)] px-3 py-2 text-sm text-[var(--color-text-secondary)]"
                            >
                                {t("settings.model.mapping.actions.clear")}
                            </button>
                        </div>

                        <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
                            <div className="rounded-lg border border-[var(--color-border)] p-3">
                                <p className="text-xs font-semibold uppercase tracking-wider text-[var(--color-text-muted)]">
                                    {t("settings.model.mapping.available_expressions", { count: modelProfile.available_expressions.length })}
                                </p>
                                <div className="mt-2 flex flex-wrap gap-2">
                                    {modelProfile.available_expressions.map((expression) => (
                                        <div key={expression} className="flex items-center gap-2 rounded-full bg-white/5 px-2 py-1 text-xs text-[var(--color-text-secondary)]">
                                            <span>{expression}</span>
                                            <button
                                                onClick={() => handlePreviewCue(expression)}
                                                className="rounded border border-[var(--color-border)] px-1.5 py-0.5 text-[10px] text-[var(--color-text-secondary)]"
                                            >
                                                {t("common.actions.preview")}
                                            </button>
                                        </div>
                                    ))}
                                </div>
                            </div>
                            <div className="rounded-lg border border-[var(--color-border)] p-3">
                                <p className="text-xs font-semibold uppercase tracking-wider text-[var(--color-text-muted)]">
                                    {t("settings.model.mapping.available_motion_groups", { count: Object.keys(modelProfile.available_motion_groups).length })}
                                </p>
                                <div className="mt-2 flex flex-wrap gap-2">
                                    {Object.entries(modelProfile.available_motion_groups).map(([group, count]) => (
                                        <div key={group} className="flex items-center gap-2 rounded-full bg-white/5 px-2 py-1 text-xs text-[var(--color-text-secondary)]">
                                            <span>{group} ({count})</span>
                                            <button
                                                onClick={() => handlePreviewCue(group)}
                                                className="rounded border border-[var(--color-border)] px-1.5 py-0.5 text-[10px] text-[var(--color-text-secondary)]"
                                            >
                                                {t("common.actions.preview")}
                                            </button>
                                        </div>
                                    ))}
                                </div>
                            </div>
                        </div>

                        <div className="space-y-2">
                            <p className="text-xs font-semibold uppercase tracking-wider text-[var(--color-text-muted)]">
                                {t("settings.model.mapping.configured_cues", { count: Object.keys(modelProfile.cue_map).length })}
                            </p>
                            {Object.entries(modelProfile.cue_map).length === 0 && (
                                <p className="text-sm text-[var(--color-text-muted)]">{t("settings.model.mapping.empty_cues")}</p>
                            )}
                            {Object.entries(modelProfile.cue_map).map(([cue, binding]) => (
                                <div
                                    key={cue}
                                    className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-[var(--color-border)] bg-black/10 px-3 py-2"
                                >
                                    <div className="min-w-0">
                                        <p className="font-mono text-sm text-[var(--color-text-primary)]">{cue}</p>
                                        <p className="text-xs text-[var(--color-text-muted)]">
                                            {binding.expression ? t("settings.model.mapping.summary.expression", { value: binding.expression }) : t("settings.model.mapping.summary.expression_none")}
                                            {" · "}
                                            {binding.motion_group ? t("settings.model.mapping.summary.motion", { value: binding.motion_group }) : t("settings.model.mapping.summary.motion_none")}
                                            {binding.exclude_from_prompt ? ` · ${t("settings.model.mapping.summary.prompt_hidden")}` : ""}
                                        </p>
                                    </div>
                                    <div className="flex items-center gap-2">
                                        <button
                                            onClick={() => handlePreviewCue(cue)}
                                            className="rounded-md border border-[var(--color-border)] px-2 py-1 text-xs text-[var(--color-text-secondary)]"
                                        >
                                            {t("common.actions.preview")}
                                        </button>
                                        <button
                                            onClick={() => handleEditCue(cue, binding)}
                                            className="rounded-md border border-[var(--color-border)] px-2 py-1 text-xs text-[var(--color-text-secondary)]"
                                        >
                                            {t("common.actions.edit")}
                                        </button>
                                        <button
                                            onClick={() => handleDeleteCue(cue)}
                                            className="rounded-md border border-red-500/40 px-2 py-1 text-xs text-red-300"
                                        >
                                            {t("common.actions.delete")}
                                        </button>
                                    </div>
                                </div>
                            ))}
                        </div>

                        <div className="space-y-3 rounded-lg border border-[var(--color-border)] p-3">
                            <div>
                                <p className="text-xs font-semibold uppercase tracking-wider text-[var(--color-text-muted)]">
                                    {t("settings.model.mapping.prompt_exclusions.title")}
                                </p>
                                <p className="mt-1 text-xs text-[var(--color-text-muted)]">
                                    {t("settings.model.mapping.prompt_exclusions.desc")}
                                </p>
                            </div>

                            <div className="flex flex-col gap-3 md:flex-row">
                                <Select
                                    value={excludedPromptCue}
                                    onChange={setExcludedPromptCue}
                                    className="flex-1"
                                    placeholder={t("settings.model.mapping.prompt_exclusions.select_cue")}
                                    options={[
                                        { value: "", label: t("settings.model.mapping.prompt_exclusions.select_cue") },
                                        ...includablePromptCueNames.map((cue) => ({ value: cue, label: cue })),
                                    ]}
                                />
                                <button
                                    onClick={async () => {
                                        const cue = excludedPromptCue.trim();
                                        if (!cue) return;
                                        await handleToggleCuePrompt(cue, true);
                                        setExcludedPromptCue("");
                                    }}
                                    disabled={!excludedPromptCue.trim()}
                                    className="rounded-lg border border-[var(--color-border)] px-3 py-2 text-sm text-[var(--color-text-secondary)] disabled:cursor-not-allowed disabled:opacity-50"
                                >
                                    {t("settings.model.mapping.actions.exclude_from_prompt")}
                                </button>
                            </div>

                            <div className="space-y-2">
                                {excludedPromptCueNames.length === 0 && (
                                    <p className="text-sm text-[var(--color-text-muted)]">
                                        {t("settings.model.mapping.prompt_exclusions.empty")}
                                    </p>
                                )}
                                {excludedPromptCueNames.map((cue) => (
                                    <div
                                        key={cue}
                                        className="flex items-center justify-between gap-3 rounded-lg border border-[var(--color-border)] bg-black/10 px-3 py-2"
                                    >
                                        <span className="font-mono text-sm text-[var(--color-text-primary)]">{cue}</span>
                                        <button
                                            onClick={() => handleToggleCuePrompt(cue, false)}
                                            className="rounded-md border border-[var(--color-border)] px-2 py-1 text-xs text-[var(--color-text-secondary)]"
                                        >
                                            {t("settings.model.mapping.actions.include_in_prompt")}
                                        </button>
                                    </div>
                                ))}
                            </div>
                        </div>

                        <div className="space-y-3 rounded-lg border border-[var(--color-border)] p-3">
                            <div>
                                <p className="text-xs font-semibold uppercase tracking-wider text-[var(--color-text-muted)]">
                                    {t("settings.model.mapping.interactions.title")}
                                </p>
                                <p className="mt-1 text-xs text-[var(--color-text-muted)]">
                                    {t("settings.model.mapping.interactions.desc")}
                                </p>
                            </div>

                            <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
                                <Select
                                    value={interactionGesture}
                                    onChange={(v) => setInteractionGesture(v as (typeof INTERACTION_GESTURES)[number]["value"])}
                                    options={INTERACTION_GESTURES.map((gesture) => ({ value: gesture.value, label: interactionGestureLabel(gesture.value) }))}
                                />
                                <Select
                                    value={interactionArea}
                                    onChange={setInteractionArea}
                                    options={interactionAreaOptions.map((area) => ({ value: area, label: interactionAreaLabel(area) }))}
                                />
                                <Select
                                    value={interactionCue}
                                    onChange={setInteractionCue}
                                    options={[
                                        { value: "", label: t("settings.model.mapping.options.select_cue") },
                                        ...availableCueNames.map((cue) => ({ value: cue, label: cue })),
                                    ]}
                                />
                            </div>

                            <div className="flex items-center gap-2">
                                <button
                                    onClick={handleSaveInteractionMapping}
                                    disabled={!interactionCue.trim()}
                                    className="rounded-lg bg-[var(--color-accent)] px-3 py-2 text-sm font-medium text-white disabled:cursor-not-allowed disabled:opacity-50"
                                >
                                    {t("settings.model.mapping.actions.save_interaction")}
                                </button>
                                <button
                                    onClick={resetInteractionDraft}
                                    className="rounded-lg border border-[var(--color-border)] px-3 py-2 text-sm text-[var(--color-text-secondary)]"
                                >
                                    {t("settings.model.mapping.actions.clear")}
                                </button>
                            </div>

                            <div className="space-y-2">
                                <p className="text-xs font-semibold uppercase tracking-wider text-[var(--color-text-muted)]">
                                    {t("settings.model.mapping.interactions.configured", { count: interactionEntries.length })}
                                </p>
                                {interactionEntries.length === 0 && (
                                    <p className="text-sm text-[var(--color-text-muted)]">{t("settings.model.mapping.interactions.empty")}</p>
                                )}
                                {interactionEntries.map(([key, cue]) => (
                                    <div
                                        key={key}
                                        className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-[var(--color-border)] bg-black/10 px-3 py-2"
                                    >
                                        <div className="min-w-0">
                                            <p className="font-mono text-sm text-[var(--color-text-primary)]">{key}</p>
                                            <p className="text-xs text-[var(--color-text-muted)]">{t("settings.model.mapping.summary.cue", { value: cue })}</p>
                                        </div>
                                        <div className="flex items-center gap-2">
                                            <button
                                                onClick={() => handleEditInteractionMapping(key, cue)}
                                                className="rounded-md border border-[var(--color-border)] px-2 py-1 text-xs text-[var(--color-text-secondary)]"
                                            >
                                                {t("common.actions.edit")}
                                            </button>
                                            <button
                                                onClick={() => handleDeleteInteractionMapping(key)}
                                                className="rounded-md border border-red-500/40 px-2 py-1 text-xs text-red-300"
                                            >
                                                {t("common.actions.delete")}
                                            </button>
                                        </div>
                                    </div>
                                ))}
                            </div>
                        </div>

                    </>
                )}
            </div>
        </div>
    );
}
