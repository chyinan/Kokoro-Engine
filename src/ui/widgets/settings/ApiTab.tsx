/**
 * ApiTab — Multi-provider LLM configuration.
 *
 * Manages OpenAI and Ollama providers through backend `LlmConfig`.
 * Provider-specific fields shown/hidden based on active provider type.
 */
import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { clsx } from "clsx";
import { RefreshCw, Check, AlertCircle, Save, Trash2, Plus } from "lucide-react";
import { motion } from "framer-motion";
import { inputClasses, labelClasses } from "../../styles/settings-primitives";
import { Select } from "@/components/ui/select";
import { useTranslation } from "react-i18next";
import {
    fetchModels,
    getLlmConfig,
    saveLlmConfig,
    listOllamaModels,
    getLlamaCppStatus,
    getContextSettings,
    setContextSettings as saveContextSettings,
    type LlmConfig,
    type LlmProviderConfig,
    type LlmPreset,
    type ContextSettings,
} from "../../../lib/kokoro-bridge";

export interface ApiTabProps {
    visionEnabled: boolean;
    onVisionEnabledChange: (v: boolean) => void;
    initialConfig?: LlmConfig | null;
    onConfigSaved?: (config: LlmConfig) => void;
    onConfigChange?: (config: LlmConfig) => void;
}

function enableProvider(config: LlmConfig, providerId?: string | null): LlmConfig {
    if (!providerId) return config;

    let changed = false;
    const providers = config.providers.map((provider) => {
        if (provider.id !== providerId || provider.enabled) {
            return provider;
        }
        changed = true;
        return { ...provider, enabled: true };
    });

    return changed ? { ...config, providers } : config;
}

function normalizeSelectedProviders(config: LlmConfig): LlmConfig {
    let next = enableProvider(config, config.active_provider);
    next = enableProvider(next, next.system_provider);
    return next;
}

type SupportedProviderType = "openai" | "ollama" | "llama_cpp";

const LLAMA_CPP_CURRENT_MODEL_KEY = "llama_cpp_current_model";
const LLAMA_CPP_CONTEXT_LENGTH_KEY = "llama_cpp_context_length";

function buildProviderId(providerType: SupportedProviderType, providers: LlmProviderConfig[]): string {
    const baseId = providerType === "llama_cpp" ? "llama-cpp" : providerType;
    if (!providers.some((provider) => provider.id === baseId)) {
        return baseId;
    }

    let suffix = 2;
    while (providers.some((provider) => provider.id === `${baseId}-${suffix}`)) {
        suffix += 1;
    }
    return `${baseId}-${suffix}`;
}

function sanitizeProviderExtra(
    providerType: SupportedProviderType,
    extra?: Record<string, unknown>,
): Record<string, unknown> {
    const nextExtra = { ...(extra || {}) };
    if (providerType !== "llama_cpp") {
        delete nextExtra[LLAMA_CPP_CURRENT_MODEL_KEY];
        delete nextExtra[LLAMA_CPP_CONTEXT_LENGTH_KEY];
    }
    return nextExtra;
}

function getDefaultBaseUrl(providerType: SupportedProviderType): string {
    switch (providerType) {
        case "ollama":
            return "http://localhost:11434";
        case "llama_cpp":
            return "http://127.0.0.1:8080";
        default:
            return "https://api.openai.com/v1";
    }
}

function getDefaultModel(providerType: SupportedProviderType): string {
    switch (providerType) {
        case "ollama":
            return "llama3";
        case "llama_cpp":
            return "";
        default:
            return "gpt-4";
    }
}

function normalizeProviderForType(
    provider: LlmProviderConfig,
    providerType: SupportedProviderType,
): LlmProviderConfig {
    const previousType = (provider.provider_type as SupportedProviderType) || "openai";
    const previousDefaultBaseUrl = getDefaultBaseUrl(previousType);
    const nextDefaultBaseUrl = getDefaultBaseUrl(providerType);
    const previousDefaultModel = getDefaultModel(previousType);
    const nextDefaultModel = getDefaultModel(providerType);
    const baseUrl =
        !provider.base_url || (previousType !== providerType && provider.base_url === previousDefaultBaseUrl)
            ? nextDefaultBaseUrl
            : provider.base_url;
    const model =
        provider.model === undefined || provider.model === null || provider.model === ""
            ? nextDefaultModel
            : previousType !== providerType && provider.model === previousDefaultModel
                ? nextDefaultModel
                : provider.model;
    const base = {
        ...provider,
        provider_type: providerType,
        base_url: baseUrl,
        model,
        extra: sanitizeProviderExtra(providerType, provider.extra),
    };

    if (providerType === "openai") {
        return {
            ...base,
            api_key_env: provider.api_key_env || "OPENAI_API_KEY",
        };
    }

    if (providerType === "ollama") {
        return {
            ...base,
            api_key: undefined,
            api_key_env: undefined,
        };
    }

    return {
        ...base,
        api_key: undefined,
        api_key_env: undefined,
    };
}

function createProvider(providerType: SupportedProviderType, providers: LlmProviderConfig[]): LlmProviderConfig {
    return normalizeProviderForType(
        {
            id: buildProviderId(providerType, providers),
            provider_type: providerType,
            enabled: true,
            supports_native_tools: true,
            api_key: undefined,
            api_key_env: undefined,
            base_url: undefined,
            model: undefined,
            extra: {},
        },
        providerType,
    );
}

function getProviderTypeLabel(providerType: string): string {
    switch (providerType) {
        case "ollama":
            return "Ollama";
        case "llama_cpp":
            return "llama.cpp";
        default:
            return "OpenAI-Compatible";
    }
}

function getProviderLocationLabel(providerType: string): string {
    return providerType === "openai" ? "Cloud" : "Local";
}

function getProviderExtraString(provider: LlmProviderConfig, key: string): string | undefined {
    const value = provider.extra?.[key];
    return typeof value === "string" && value.trim() !== "" ? value : undefined;
}

function getProviderExtraNumber(provider: LlmProviderConfig, key: string): number | undefined {
    const value = provider.extra?.[key];
    if (typeof value === "number" && Number.isFinite(value)) {
        return value;
    }
    if (typeof value === "string") {
        const parsed = Number(value);
        return Number.isFinite(parsed) ? parsed : undefined;
    }
    return undefined;
}

export default function ApiTab({ visionEnabled, onVisionEnabledChange, initialConfig = null, onConfigSaved, onConfigChange }: ApiTabProps) {
    const { t } = useTranslation();
    const [config, setConfigRaw] = useState<LlmConfig | null>(initialConfig);
    const onConfigChangeRef = useRef(onConfigChange);
    onConfigChangeRef.current = onConfigChange;
    const setConfig = useCallback((cfg: LlmConfig | null | ((prev: LlmConfig | null) => LlmConfig | null)) => {
        setConfigRaw(prev => {
            const next = typeof cfg === 'function' ? cfg(prev) : cfg;
            if (next) onConfigChangeRef.current?.(next);
            return next;
        });
    }, []);
    const [loading, setLoading] = useState(initialConfig === null);
    const [saving, setSaving] = useState(false);
    const [saved, setSaved] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [availableModels, setAvailableModels] = useState<string[]>([]);
    const [isLoadingModels, setIsLoadingModels] = useState(false);
    const [selectedPresetId, setSelectedPresetId] = useState<string>("");
    const [contextSettings, setContextSettings] = useState<ContextSettings>({
        strategy: "window",
        max_message_chars: 2000,
    });
    // Load config from backend on mount
    useEffect(() => {
        if (initialConfig) {
            setConfig(normalizeSelectedProviders(initialConfig));
            setLoading(false);
        }

        if (!initialConfig) {
            getLlmConfig()
                .then((cfg) => {
                    setConfig(normalizeSelectedProviders(cfg));
                    setLoading(false);
                })
                .catch((e) => {
                    console.error("Failed to load LLM config:", e);
                    setError(typeof e === 'string' ? e : ((e as any)?.message ?? JSON.stringify(e)));
                    setLoading(false);
                });
        }

        getContextSettings()
            .then(setContextSettings)
            .catch((e) => console.error("Failed to load context settings:", e));
    }, [initialConfig]);

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

    const updateActiveProviderExtra = useCallback(
        (updates: Record<string, unknown | undefined>) => {
            if (!activeProvider) return;
            const nextExtra = { ...(activeProvider.extra || {}) };
            for (const [key, value] of Object.entries(updates)) {
                if (value === undefined || value === null || value === "") {
                    delete nextExtra[key];
                } else {
                    nextExtra[key] = value;
                }
            }
            updateActiveProvider({ extra: nextExtra });
        },
        [activeProvider, updateActiveProvider]
    );

    const saveDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    const handleContextSettingsChange = useCallback(
        (updates: Partial<ContextSettings>) => {
            const updated = { ...contextSettings, ...updates };
            setContextSettings(updated);
            // Debounce IPC save to avoid firing on every keystroke
            if (saveDebounceRef.current) clearTimeout(saveDebounceRef.current);
            saveDebounceRef.current = setTimeout(() => {
                saveContextSettings(updated).catch((e) => console.error("Failed to save context settings:", e));
            }, 400);
        },
        [contextSettings]
    );

    const handleSavePreset = useCallback(() => {
        if (!config) return;
        const normalizedConfig = normalizeSelectedProviders(config);
        if (normalizedConfig !== config) {
            setConfig(normalizedConfig);
        }
        const existing = selectedPresetId
            ? normalizedConfig.presets?.find((p) => p.id === selectedPresetId)
            : null;
        const defaultName = existing?.name || "";
        const name = window.prompt(t("settings.api.preset.name_prompt"), defaultName);
        if (!name) return;

        const preset: LlmPreset = {
            id: existing?.id || crypto.randomUUID(),
            name,
            active_provider: normalizedConfig.active_provider,
            system_provider: normalizedConfig.system_provider,
            system_model: normalizedConfig.system_model,
            providers: JSON.parse(JSON.stringify(normalizedConfig.providers)),
        };

        const presets = [...(normalizedConfig.presets || [])];
        const idx = presets.findIndex((p) => p.id === preset.id);
        if (idx >= 0) {
            presets[idx] = preset;
        } else {
            presets.push(preset);
        }

        const updated = { ...normalizedConfig, presets };
        setConfig(updated);
        setSelectedPresetId(preset.id);
        saveLlmConfig(updated).catch((e) => setError(typeof e === 'string' ? e : ((e as any)?.message ?? JSON.stringify(e))));
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

            const updated = normalizeSelectedProviders({
                ...config,
                active_provider: preset.active_provider,
                system_provider: preset.system_provider,
                system_model: preset.system_model,
                providers: Array.from(providerMap.values()),
            });
            setConfig(updated);
            saveLlmConfig(updated).catch((e) => setError(typeof e === 'string' ? e : ((e as any)?.message ?? JSON.stringify(e))));
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
        saveLlmConfig(updated).catch((e) => setError(typeof e === 'string' ? e : ((e as any)?.message ?? JSON.stringify(e))));
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

            let updatedConfig = normalizeSelectedProviders({
                ...config,
                providers: Array.from(providerMap.values()),
            });

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
            onConfigSaved?.(updatedConfig);
            setSaved(true);
            setTimeout(() => setSaved(false), 2000);
        } catch (e) {
            console.error("Failed to save LLM config:", e);
            setError(typeof e === 'string' ? e : ((e as any)?.message ?? JSON.stringify(e)));
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
            } else if (activeProvider.provider_type === "llama_cpp") {
                const baseUrl = activeProvider.base_url || "http://127.0.0.1:8080";
                const status = await getLlamaCppStatus(baseUrl);
                const resolvedModel =
                    status.current_model || status.available_models[0] || activeProvider.model || "";
                setAvailableModels(status.available_models);
                updateActiveProvider({
                    model: resolvedModel,
                    extra: {
                        ...(activeProvider.extra || {}),
                        ...(status.current_model
                            ? { [LLAMA_CPP_CURRENT_MODEL_KEY]: status.current_model }
                            : {}),
                        ...(typeof status.context_length === "number"
                            ? { [LLAMA_CPP_CONTEXT_LENGTH_KEY]: status.context_length }
                            : {}),
                    },
                });
            } else {
                // OpenAI-compatible: use /v1/models
                const apiKey = activeProvider.api_key || "";
                const baseUrl = activeProvider.base_url || "https://api.openai.com/v1";
                const models = await fetchModels(baseUrl, apiKey);
                setAvailableModels(models);
            }
        } catch (e) {
            console.error("Failed to fetch models:", e);
            setError(typeof e === 'string' ? e : ((e as any)?.message ?? JSON.stringify(e)));
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
                {t("settings.api.loading_config", { defaultValue: "Loading LLM config..." })}
            </div>
        );
    }

    if (!config || !activeProvider) {
        return (
            <div className="text-center py-8 text-red-400">
                <AlertCircle size={20} className="mx-auto mb-2" />
                {t("settings.api.load_failed", { defaultValue: "Failed to load LLM configuration" })}
            </div>
        );
    }

    const isOllama = activeProvider.provider_type === "ollama";
    const isLlamaCpp = activeProvider.provider_type === "llama_cpp";
    const showApiKey = activeProvider.provider_type === "openai";
    const configuredContextLength = getProviderExtraNumber(activeProvider, LLAMA_CPP_CONTEXT_LENGTH_KEY);
    const detectedCurrentModel = getProviderExtraString(activeProvider, LLAMA_CPP_CURRENT_MODEL_KEY);
    const modelFetchDisabled =
        isLoadingModels || (activeProvider.provider_type === "openai" && !activeProvider.api_key);

    return (
        <div className="space-y-4">
            {/* Preset Selector */}
            <div>
                <label className={labelClasses}>{t("settings.api.preset.label")}</label>
                <div className="flex gap-2">
                    <Select
                        value={selectedPresetId}
                        onChange={handleLoadPreset}
                        options={[
                            { value: "", label: matchingPreset ? matchingPreset.name : t("settings.api.preset.current") },
                            ...(config.presets || []).map(p => ({ value: p.id, label: p.name })),
                        ]}
                        className="flex-1"
                    />
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
                        <div key={p.id} className="relative flex-1 group/card">
                            <button
                                onClick={() => setConfig(normalizeSelectedProviders({ ...config, active_provider: p.id }))}
                                className={clsx(
                                    "w-full px-3 py-2 text-xs rounded-lg border transition-all",
                                    config.active_provider === p.id
                                        ? "border-[var(--color-accent)] bg-[var(--color-accent)]/10 text-[var(--color-accent)]"
                                        : "border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-[var(--color-text-muted)]"
                                )}
                            >
                                <div className="font-medium capitalize">{p.id}</div>
                                <div className="text-[9px] opacity-70 mt-0.5">
                                    {getProviderLocationLabel(p.provider_type)} · {getProviderTypeLabel(p.provider_type)}
                                </div>
                            </button>
                            {config.providers.length > 1 && (
                                <button
                                    onClick={(e) => {
                                        e.stopPropagation();
                                        const remaining = config.providers.filter((x) => x.id !== p.id);
                                        const newActive = config.active_provider === p.id
                                            ? remaining[0]?.id ?? ""
                                            : config.active_provider;
                                        const newSystemProvider = config.system_provider === p.id
                                            ? undefined
                                            : config.system_provider;
                                        setConfig(normalizeSelectedProviders({
                                            ...config,
                                            providers: remaining,
                                            active_provider: newActive,
                                            system_provider: newSystemProvider,
                                        }));
                                    }}
                                    className="absolute top-1 right-2 text-[var(--color-text-muted)] hover:text-red-400 opacity-0 group-hover/card:opacity-100 transition-opacity"
                                    title={t("common.actions.delete")}
                                >
                                    <svg width="6" height="6" viewBox="0 0 8 8" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                                        <line x1="1" y1="1" x2="7" y2="7"/>
                                        <line x1="7" y1="1" x2="1" y2="7"/>
                                    </svg>
                                </button>
                            )}
                        </div>
                    ))}
                </div>
                <div className="flex flex-wrap gap-2 mt-2">
                    {(["openai", "ollama", "llama_cpp"] as const).map((providerType) => (
                        <button
                            key={providerType}
                            onClick={() => {
                                const provider = createProvider(providerType, config.providers);
                                setConfig(normalizeSelectedProviders({
                                    ...config,
                                    providers: [...config.providers, provider],
                                    active_provider: provider.id,
                                }));
                            }}
                            className="px-3 py-1.5 text-[10px] rounded-lg border border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-all flex items-center gap-1"
                        >
                            <Plus size={10} />
                            {getProviderTypeLabel(providerType)}
                        </button>
                    ))}
                </div>
            </div>

            {/* Provider ID */}
            <div>
                <label className={labelClasses}>
                    {t("settings.api.provider_id_label", { defaultValue: "Provider ID" })}
                </label>
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
                            system_provider: config.system_provider === activeProvider.id ? newId : config.system_provider,
                        });
                    }}
                    placeholder="e.g., comiai, xianyu-opus"
                    className={clsx(inputClasses, "font-mono")}
                />
                <p className="text-[9px] text-[var(--color-text-muted)] mt-1">
                    {t("settings.api.provider_id_hint", {
                        defaultValue: "Unique identifier for this provider (used in system LLM selection)",
                    })}
                </p>
            </div>

            {/* API Key (OpenAI only) */}
            {showApiKey && (
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
                    {isOllama
                        ? t("settings.api.ollama_url")
                        : isLlamaCpp
                            ? t("settings.api.llama_cpp_url", { defaultValue: "llama.cpp Server URL" })
                            : t("settings.api.endpoint_url")}
                </label>
                <input
                    type="url"
                    value={activeProvider.base_url || ""}
                    onChange={(e) => updateActiveProvider({ base_url: e.target.value })}
                    placeholder={
                        isOllama
                            ? "http://localhost:11434"
                            : isLlamaCpp
                                ? "http://127.0.0.1:8080"
                                : "https://api.openai.com/v1"
                    }
                    className={clsx(inputClasses, "font-mono")}
                />
            </div>

            {/* Model */}
            <div>
                <div className="flex justify-between items-center mb-2">
                    <label className={labelClasses.replace("mb-2", "mb-0")}>{t("settings.api.model_label")}</label>
                    <button
                        onClick={handleFetchModels}
                        disabled={modelFetchDisabled}
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
                        placeholder={
                            isOllama
                                ? "llama3"
                                : isLlamaCpp
                                    ? "Qwen2.5-7B-Instruct"
                                    : "gpt-4"
                        }
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

            {isLlamaCpp && (
                <>
                    <div>
                        <label className={labelClasses}>
                            {t("settings.api.current_model_label", { defaultValue: "Current Model" })}
                        </label>
                        <div className={clsx(inputClasses, "font-mono text-[var(--color-text-main)]")}>
                            {detectedCurrentModel || t("settings.api.current_model_empty", { defaultValue: "Not detected yet" })}
                        </div>
                        <p className="text-[9px] text-[var(--color-text-muted)] mt-1">
                            {t("settings.api.current_model_hint", { defaultValue: "Fetched from the active llama.cpp server." })}
                        </p>
                    </div>

                    <div>
                        <label className={labelClasses}>
                            {t("settings.api.context_length_label", { defaultValue: "Context Length" })}
                        </label>
                        <input
                            type="number"
                            min={256}
                            step={256}
                            value={configuredContextLength ?? ""}
                            onChange={(e) => {
                                const value = e.target.value.trim();
                                updateActiveProviderExtra({
                                    [LLAMA_CPP_CONTEXT_LENGTH_KEY]: value ? Number(value) : undefined,
                                });
                            }}
                            placeholder="16384"
                            className={clsx(inputClasses, "font-mono [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none")}
                        />
                        <p className="text-[9px] text-[var(--color-text-muted)] mt-1">
                            {t("settings.api.context_length_hint", {
                                defaultValue: 'You can enter it manually or use "Fetch Available" to read it from llama.cpp.',
                            })}
                        </p>
                    </div>
                </>
            )}

            <div>
                <div className="flex items-start justify-between gap-3">
                    <div>
                        <label className={labelClasses.replace("mb-2", "mb-0")}>
                            {t("settings.api.native_tools.label")}
                        </label>
                        <p className="text-sm text-[var(--color-text-main)] mt-1">
                            {t("settings.api.native_tools.toggle")}
                        </p>
                        <p className="text-[9px] text-[var(--color-text-muted)] mt-1">
                            {t("settings.api.native_tools.desc")}
                        </p>
                    </div>
                    <button
                        type="button"
                        aria-pressed={activeProvider.supports_native_tools ?? true}
                        onClick={() =>
                            updateActiveProvider({
                                supports_native_tools: !(activeProvider.supports_native_tools ?? true),
                            })
                        }
                        className={clsx(
                            "w-10 h-6 rounded-full transition-colors relative shrink-0 mt-5",
                            (activeProvider.supports_native_tools ?? true)
                                ? "bg-[var(--color-accent)]"
                                : "bg-[var(--color-border)]"
                        )}
                    >
                        <motion.div
                            animate={{ x: (activeProvider.supports_native_tools ?? true) ? 18 : 2 }}
                            transition={{ type: "spring", stiffness: 500, damping: 30 }}
                            className="absolute top-1 w-4 h-4 rounded-full bg-white"
                        />
                    </button>
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
                        <Select
                            value={config.system_provider || ""}
                            onChange={(v) => setConfig(normalizeSelectedProviders({ ...config, system_provider: v || undefined }))}
                            options={[
                                { value: "", label: t("settings.api.system_llm.same_as_active", { provider: config.active_provider }) },
                                ...allAvailableProviders.map(p => ({
                                    value: p.id,
                                    label: `${p.id} (${p.provider_type})`,
                                })),
                            ]}
                        />
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
                            ? "border-[var(--color-accent)]/50 bg-[var(--color-accent)]/10 text-[var(--color-accent)]"
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

            {/* Context Management */}
            <div className="pt-4 border-t border-[var(--color-border)]">
                <div className="mb-3">
                    <label className="text-xs font-medium text-[var(--color-text-main)] block mb-1">
                        {t("settings.api.context.title")}
                    </label>
                    <p className="text-[10px] text-[var(--color-text-muted)]">
                        {t("settings.api.context.desc")}
                    </p>
                </div>
                <div className="flex gap-2 mb-3">
                    {(["window", "summary"] as const).map((s) => (
                        <button
                            key={s}
                            onClick={() => handleContextSettingsChange({ strategy: s })}
                            className={clsx(
                                "flex-1 px-3 py-2 text-xs rounded-lg border transition-all",
                                contextSettings.strategy === s
                                    ? "border-[var(--color-accent)] bg-[var(--color-accent)]/10 text-[var(--color-accent)]"
                                    : "border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-[var(--color-text-muted)]"
                            )}
                        >
                            <div className="font-medium">
                                {t(`settings.api.context.strategy_${s}`)}
                            </div>
                            <div className="text-[9px] opacity-70 mt-0.5">
                                {t(`settings.api.context.strategy_${s}_desc`)}
                            </div>
                        </button>
                    ))}
                </div>
                <div>
                    <label className="text-[10px] uppercase tracking-wider text-[var(--color-text-muted)] font-semibold mb-1 block">
                        {t("settings.api.context.max_chars_label")}
                    </label>
                    <input
                        type="number"
                        min={100}
                        max={20000}
                        value={contextSettings.max_message_chars}
                        onChange={(e) => handleContextSettingsChange({ max_message_chars: Number(e.target.value) })}
                        className={clsx(inputClasses, "font-mono [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none")}
                    />
                    <p className="text-[9px] text-[var(--color-text-muted)] mt-1">
                        {t("settings.api.context.max_chars_hint")}
                    </p>
                </div>
            </div>

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
                            visionEnabled ? "bg-[var(--color-accent)]" : "bg-[var(--color-border)]"
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
