/**
 * ApiTab — Multi-provider LLM configuration.
 *
 * Manages OpenAI and Ollama providers through backend `LlmConfig`.
 * Provider-specific fields shown/hidden based on active provider type.
 */
import { useState, useEffect, useCallback, useMemo } from "react";
import { clsx } from "clsx";
import { RefreshCw, Check, AlertCircle, Save, Trash2 } from "lucide-react";
import { motion } from "framer-motion";
import { inputClasses, labelClasses } from "../../styles/settings-primitives";
import { useTranslation } from "react-i18next";
import {
    fetchModels,
    getLlmConfig,
    saveLlmConfig,
    listOllamaModels,
    type LlmConfig,
    type LlmProviderConfig,
    type LlmPreset,
} from "../../../lib/kokoro-bridge";

export interface ApiTabProps {
    visionEnabled: boolean;
    onVisionEnabledChange: (v: boolean) => void;
}

export default function ApiTab({ visionEnabled, onVisionEnabledChange }: ApiTabProps) {
    const { t } = useTranslation();
    const [config, setConfig] = useState<LlmConfig | null>(null);
    const [loading, setLoading] = useState(true);
    const [saving, setSaving] = useState(false);
    const [saved, setSaved] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [availableModels, setAvailableModels] = useState<string[]>([]);
    const [isLoadingModels, setIsLoadingModels] = useState(false);
    const [selectedPresetId, setSelectedPresetId] = useState<string>("");

    // Load config from backend on mount
    useEffect(() => {
        getLlmConfig()
            .then((cfg) => {
                setConfig(cfg);
                setLoading(false);
            })
            .catch((e) => {
                console.error("Failed to load LLM config:", e);
                setError(String(e));
                setLoading(false);
            });
    }, []);

    const activeProvider = config
        ? config.providers.find((p) => p.id === config.active_provider) ?? config.providers[0]
        : null;

    // Collect all unique providers from current config and all presets
    const allAvailableProviders = useMemo(() => {
        if (!config) return [];
        const providerMap = new Map<string, LlmProviderConfig>();

        // Add current providers
        config.providers.forEach(p => providerMap.set(p.id, p));

        // Add providers from all presets
        (config.presets || []).forEach(preset => {
            preset.providers.forEach(p => {
                if (!providerMap.has(p.id)) {
                    providerMap.set(p.id, p);
                }
            });
        });

        return Array.from(providerMap.values());
    }, [config]);

    const updateActiveProvider = useCallback(
        (updates: Partial<LlmProviderConfig>) => {
            if (!config || !activeProvider) return;
            setConfig({
                ...config,
                providers: config.providers.map((p) =>
                    p.id === activeProvider.id ? { ...p, ...updates } : p
                ),
            });
        },
        [config, activeProvider]
    );

    const handleSavePreset = useCallback(() => {
        if (!config) return;
        const existing = selectedPresetId
            ? config.presets?.find((p) => p.id === selectedPresetId)
            : null;
        const defaultName = existing?.name || "";
        const name = window.prompt(t("settings.api.preset.name_prompt"), defaultName);
        if (!name) return;

        const preset: LlmPreset = {
            id: existing?.id || crypto.randomUUID(),
            name,
            active_provider: config.active_provider,
            system_provider: config.system_provider,
            system_model: config.system_model,
            providers: JSON.parse(JSON.stringify(config.providers)),
        };

        const presets = [...(config.presets || [])];
        const idx = presets.findIndex((p) => p.id === preset.id);
        if (idx >= 0) {
            presets[idx] = preset;
        } else {
            presets.push(preset);
        }

        const updated = { ...config, presets };
        setConfig(updated);
        setSelectedPresetId(preset.id);
        saveLlmConfig(updated).catch((e) => setError(String(e)));
    }, [config, selectedPresetId, t]);

    const handleLoadPreset = useCallback(
        (presetId: string) => {
            if (!config) return;
            setSelectedPresetId(presetId);
            if (!presetId) return;

            const preset = config.presets?.find((p) => p.id === presetId);
            if (!preset) return;

            // Merge providers: keep all existing providers, update/add from preset
            const providerMap = new Map<string, LlmProviderConfig>();

            // Start with current providers
            config.providers.forEach(p => providerMap.set(p.id, p));

            // Add/update from all presets (to ensure all providers are available)
            (config.presets || []).forEach(ps => {
                ps.providers.forEach(p => {
                    if (!providerMap.has(p.id)) {
                        providerMap.set(p.id, JSON.parse(JSON.stringify(p)));
                    }
                });
            });

            const updated: LlmConfig = {
                ...config,
                active_provider: preset.active_provider,
                system_provider: preset.system_provider,
                system_model: preset.system_model,
                providers: Array.from(providerMap.values()),
            };
            setConfig(updated);
            saveLlmConfig(updated).catch((e) => setError(String(e)));
        },
        [config]
    );

    const handleDeletePreset = useCallback(() => {
        if (!config || !selectedPresetId) return;
        const preset = config.presets?.find((p) => p.id === selectedPresetId);
        if (!preset) return;
        if (!window.confirm(`${t("settings.api.preset.delete")} "${preset.name}"?`)) return;

        const presets = (config.presets || []).filter((p) => p.id !== selectedPresetId);
        const updated = { ...config, presets };
        setConfig(updated);
        setSelectedPresetId("");
        saveLlmConfig(updated).catch((e) => setError(String(e)));
    }, [config, selectedPresetId, t]);

    const handleSave = async () => {
        if (!config) return;
        setSaving(true);
        setError(null);
        try {
            // Merge all providers from all presets to ensure they're all available
            const providerMap = new Map<string, LlmProviderConfig>();

            // Start with current providers
            config.providers.forEach(p => providerMap.set(p.id, p));

            // Add providers from all presets
            (config.presets || []).forEach(preset => {
                preset.providers.forEach(p => {
                    if (!providerMap.has(p.id)) {
                        providerMap.set(p.id, JSON.parse(JSON.stringify(p)));
                    }
                });
            });

            let updatedConfig = {
                ...config,
                providers: Array.from(providerMap.values()),
            };

            // If a preset is selected, also update that preset with current config
            if (selectedPresetId) {
                const presets = [...(updatedConfig.presets || [])];
                const idx = presets.findIndex((p) => p.id === selectedPresetId);
                if (idx >= 0) {
                    presets[idx] = {
                        ...presets[idx],
                        active_provider: updatedConfig.active_provider,
                        system_provider: updatedConfig.system_provider,
                        system_model: updatedConfig.system_model,
                        providers: JSON.parse(JSON.stringify(updatedConfig.providers)),
                    };
                    updatedConfig = { ...updatedConfig, presets };
                }
            }

            await saveLlmConfig(updatedConfig);
            setConfig(updatedConfig);
            setSaved(true);
            setTimeout(() => setSaved(false), 2000);
        } catch (e) {
            console.error("Failed to save LLM config:", e);
            setError(String(e));
        } finally {
            setSaving(false);
        }
    };

    const handleFetchModels = async () => {
        if (!activeProvider) return;
        setIsLoadingModels(true);
        try {
            if (activeProvider.provider_type === "ollama") {
                const baseUrl = activeProvider.base_url || "http://localhost:11434";
                const models = await listOllamaModels(baseUrl);
                setAvailableModels(models.map((m) => m.name));
            } else {
                // OpenAI-compatible: use /v1/models
                const apiKey = activeProvider.api_key || "";
                const baseUrl = activeProvider.base_url || "https://api.openai.com/v1";
                const models = await fetchModels(baseUrl, apiKey);
                setAvailableModels(models);
            }
        } catch (e) {
            console.error("Failed to fetch models:", e);
            setError(String(e));
        } finally {
            setIsLoadingModels(false);
        }
    };

    // Check if current config matches a preset (must be before any conditional returns)
    const matchingPreset = useMemo(() => {
        if (!config || selectedPresetId) return null;

        return (config.presets || []).find(preset => {
            // Check basic fields
            if (preset.active_provider !== config.active_provider ||
                preset.system_provider !== config.system_provider ||
                preset.system_model !== config.system_model) {
                return false;
            }

            // Deep compare active provider config
            const currentActiveProvider = config.providers.find(p => p.id === config.active_provider);
            const presetActiveProvider = preset.providers.find(p => p.id === preset.active_provider);

            if (!currentActiveProvider || !presetActiveProvider) return false;

            // Compare key fields of active provider
            return currentActiveProvider.model === presetActiveProvider.model &&
                   currentActiveProvider.base_url === presetActiveProvider.base_url &&
                   currentActiveProvider.api_key === presetActiveProvider.api_key;
        });
    }, [config, selectedPresetId]);

    if (loading) {
        return (
            <div className="flex items-center justify-center py-8 text-[var(--color-text-muted)]">
                <RefreshCw size={14} className="animate-spin mr-2" />
                Loading LLM config...
            </div>
        );
    }

    if (!config || !activeProvider) {
        return (
            <div className="text-center py-8 text-red-400">
                <AlertCircle size={20} className="mx-auto mb-2" />
                Failed to load LLM configuration
            </div>
        );
    }

    const isOllama = activeProvider.provider_type === "ollama";

    return (
        <div className="space-y-4">
            {/* Preset Selector */}
            <div>
                <label className={labelClasses}>{t("settings.api.preset.label")}</label>
                <div className="flex gap-2">
                    <select
                        value={selectedPresetId}
                        onChange={(e) => handleLoadPreset(e.target.value)}
                        className={clsx(inputClasses, "flex-1 py-1.5 px-2")}
                    >
                        <option value="">
                            {matchingPreset ? matchingPreset.name : t("settings.api.preset.current")}
                        </option>
                        {(config.presets || []).map((p) => (
                            <option key={p.id} value={p.id}>{p.name}</option>
                        ))}
                    </select>
                    <button
                        onClick={handleSavePreset}
                        className="px-3 py-1.5 text-xs rounded-lg border border-[var(--color-accent)] text-[var(--color-accent)] hover:bg-[var(--color-accent)]/10 transition-all flex items-center gap-1"
                        title={t("settings.api.preset.save")}
                    >
                        <Save size={12} />
                    </button>
                    <button
                        onClick={handleDeletePreset}
                        disabled={!selectedPresetId}
                        className="px-3 py-1.5 text-xs rounded-lg border border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-red-400 hover:text-red-400 disabled:opacity-30 disabled:pointer-events-none transition-all flex items-center gap-1"
                        title={t("settings.api.preset.delete")}
                    >
                        <Trash2 size={12} />
                    </button>
                </div>
            </div>

            {/* Provider Selector */}
            <div>
                <label className={labelClasses}>{t("settings.api.provider_label")}</label>
                <div className="flex gap-2">
                    {config.providers.map((p) => (
                        <button
                            key={p.id}
                            onClick={() => setConfig({ ...config, active_provider: p.id })}
                            className={clsx(
                                "flex-1 px-3 py-2 text-xs rounded-lg border transition-all",
                                config.active_provider === p.id
                                    ? "border-[var(--color-accent)] bg-[var(--color-accent)]/10 text-[var(--color-accent)]"
                                    : "border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-[var(--color-text-muted)]"
                            )}
                        >
                            <div className="font-medium capitalize">{p.id}</div>
                            <div className="text-[9px] opacity-70 mt-0.5">
                                {p.provider_type === "ollama" ? "Local" : "Cloud"}
                            </div>
                        </button>
                    ))}
                </div>
            </div>

            {/* Provider ID */}
            <div>
                <label className={labelClasses}>Provider ID</label>
                <input
                    type="text"
                    value={activeProvider.id || ""}
                    onChange={(e) => {
                        const newId = e.target.value.trim();
                        if (!newId) return;
                        // Update the provider id
                        setConfig({
                            ...config,
                            providers: config.providers.map((p) =>
                                p.id === activeProvider.id ? { ...p, id: newId } : p
                            ),
                            active_provider: config.active_provider === activeProvider.id ? newId : config.active_provider,
                        });
                    }}
                    placeholder="e.g., comiai, xianyu-opus"
                    className={clsx(inputClasses, "font-mono")}
                />
                <p className="text-[9px] text-[var(--color-text-muted)] mt-1">
                    Unique identifier for this provider (used in system LLM selection)
                </p>
            </div>

            {/* API Key (OpenAI only) */}
            {!isOllama && (
                <div>
                    <label className={labelClasses}>{t("settings.api.api_key")}</label>
                    <input
                        type="password"
                        value={activeProvider.api_key || ""}
                        onChange={(e) => updateActiveProvider({ api_key: e.target.value })}
                        placeholder="sk-..."
                        className={clsx(inputClasses, "font-mono")}
                    />
                    {activeProvider.api_key_env && (
                        <p className="text-[9px] text-[var(--color-text-muted)] mt-1">
                            {t("settings.api.fallback_env")} <code className="text-[var(--color-accent)]">{activeProvider.api_key_env}</code>
                        </p>
                    )}
                </div>
            )}

            {/* Base URL */}
            {/* Base URL */}
            <div>
                <label className={labelClasses}>
                    {isOllama ? t("settings.api.ollama_url") : t("settings.api.endpoint_url")}
                </label>
                <input
                    type="url"
                    value={activeProvider.base_url || ""}
                    onChange={(e) => updateActiveProvider({ base_url: e.target.value })}
                    placeholder={isOllama ? "http://localhost:11434" : "https://api.openai.com/v1"}
                    className={clsx(inputClasses, "font-mono")}
                />
            </div>

            {/* Model */}
            <div>
                <div className="flex justify-between items-center mb-2">
                    <label className={labelClasses.replace("mb-2", "mb-0")}>{t("settings.api.model_label")}</label>
                    <button
                        onClick={handleFetchModels}
                        disabled={isLoadingModels || (!isOllama && !activeProvider.api_key)}
                        className="text-[10px] uppercase tracking-wider text-[var(--color-accent)] hover:underline disabled:opacity-50 flex items-center gap-1"
                    >
                        <RefreshCw size={10} className={isLoadingModels ? "animate-spin" : ""} />
                        {isLoadingModels ? t("settings.api.fetching") : t("settings.api.fetch_models")}
                    </button>
                </div>
                <div className="relative">
                    <input
                        type="text"
                        value={activeProvider.model || ""}
                        onChange={(e) => updateActiveProvider({ model: e.target.value })}
                        placeholder={isOllama ? "llama3" : "gpt-4"}
                        list="model-list"
                        className={clsx(inputClasses, "font-mono")}
                    />
                    <datalist id="model-list">
                        {availableModels.map((m) => (
                            <option key={m} value={m} />
                        ))}
                    </datalist>
                </div>
            </div>

            {/* System LLM Config */}
            <div className="pt-4 border-t border-[var(--color-border)]">
                <div className="mb-3">
                    <label className="text-xs font-medium text-[var(--color-text-main)] block mb-1">
                        {t("settings.api.system_llm.title")}
                    </label>
                    <p className="text-[10px] text-[var(--color-text-muted)]">
                        {t("settings.api.system_llm.desc")}
                    </p>
                </div>

                <div className="grid grid-cols-2 gap-3">
                    {/* System Provider Selector */}
                    <div>
                        <label className="text-[10px] uppercase tracking-wider text-[var(--color-text-muted)] font-semibold mb-1 block">
                            {t("settings.api.system_llm.provider")}
                        </label>
                        <select
                            value={config.system_provider || ""}
                            onChange={(e) => setConfig({ ...config, system_provider: e.target.value || undefined })}
                            className={clsx(inputClasses, "py-1.5 px-2")}
                        >
                            <option value="">{t("settings.api.system_llm.same_as_active", { provider: config.active_provider })}</option>
                            {allAvailableProviders.map(p => (
                                <option key={p.id} value={p.id}>
                                    {p.id} ({p.provider_type})
                                </option>
                            ))}
                        </select>
                    </div>

                    {/* System Model Override */}
                    <div>
                        <label className="text-[10px] uppercase tracking-wider text-[var(--color-text-muted)] font-semibold mb-1 block">
                            {t("settings.api.system_llm.model_override")}
                        </label>
                        <input
                            type="text"
                            value={config.system_model || ""}
                            onChange={(e) => setConfig({ ...config, system_model: e.target.value || undefined })}
                            placeholder="default"
                            className={clsx(inputClasses, "py-1.5 px-2 font-mono")}
                        />
                    </div>
                </div>
            </div>

            {/* Save button */}
            <div className="pt-2 border-t border-[var(--color-border)]">
                <button
                    onClick={handleSave}
                    disabled={saving}
                    className={clsx(
                        "w-full py-2 text-xs rounded-lg border transition-all",
                        saved
                            ? "border-emerald-500/50 bg-emerald-500/10 text-emerald-400"
                            : "border-[var(--color-accent)] bg-[var(--color-accent)]/10 text-[var(--color-accent)] hover:bg-[var(--color-accent)]/20"
                    )}
                >
                    {saving ? (
                        <span className="flex items-center justify-center gap-1.5">
                            <RefreshCw size={10} className="animate-spin" /> {t("settings.api.saving")}
                        </span>
                    ) : saved ? (
                        <span className="flex items-center justify-center gap-1.5">
                            <Check size={10} /> {t("common.actions.saved")}
                        </span>
                    ) : (
                        t("settings.api.save_config")
                    )}
                </button>
            </div>

            {/* Error display */}
            {error && (
                <div className="text-[10px] text-red-400 bg-red-400/10 px-3 py-2 rounded-lg">
                    {error}
                </div>
            )}

            {/* Vision Mode Toggle */}
            <div className="pt-2 border-t border-[var(--color-border)]">
                <div className="flex items-center justify-between">
                    <div>
                        <label className={labelClasses.replace("mb-2", "mb-0")}>{t("settings.api.vision_mode")}</label>
                        <p className="text-[10px] text-[var(--color-text-muted)] mt-0.5">
                            {t("settings.api.vision_desc")}
                        </p>
                    </div>
                    <button
                        onClick={() => onVisionEnabledChange(!visionEnabled)}
                        className={clsx(
                            "w-10 h-5 rounded-full transition-colors relative",
                            visionEnabled ? "bg-emerald-500/50" : "bg-[var(--color-border)]"
                        )}
                    >
                        <motion.div
                            animate={{ x: visionEnabled ? 20 : 2 }}
                            className="absolute top-0.5 w-4 h-4 rounded-full bg-white"
                        />
                    </button>
                </div>
            </div>
        </div>
    );
}
