import { useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { useTranslation } from "react-i18next";
import { Trash2, Plus, Wifi, RefreshCw, CheckCircle, XCircle, Image as ImageIcon, Loader2 } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { inputClasses, labelClasses } from "../styles/settings-primitives";
import { testSdConnection, generateImage } from "../../lib/kokoro-bridge";
import type { ImageGenSystemConfig, ImageGenProviderConfig } from "../../lib/kokoro-bridge";

interface ImageGenSettingsProps {
    config: ImageGenSystemConfig;
    onChange: (config: ImageGenSystemConfig) => void;
}

export default function ImageGenSettings({ config, onChange }: ImageGenSettingsProps) {
    const { t } = useTranslation();
    const [editingId, setEditingId] = useState<string | null>(null);
    const [testState, setTestState] = useState<Record<string, { loading: boolean; result?: string; error?: string; models?: string[] }>>({});
    const [genTestState, setGenTestState] = useState<Record<string, { loading: boolean; imageUrl?: string; error?: string }>>({});

    const handleTestSd = async (providerId: string, baseUrl: string) => {
        setTestState(prev => ({ ...prev, [providerId]: { loading: true } }));
        try {
            const models = await testSdConnection(baseUrl || "http://127.0.0.1:7860");
            setTestState(prev => ({
                ...prev,
                [providerId]: {
                    loading: false,
                    result: t("settings.image_gen.test.success", { count: models.length }),
                    models
                }
            }));
        } catch (e) {
            setTestState(prev => ({
                ...prev,
                [providerId]: { loading: false, error: String(e) }
            }));
        }
    };

    const handleTestGenerate = async (providerId: string) => {
        setGenTestState(prev => ({ ...prev, [providerId]: { loading: true, error: undefined, imageUrl: undefined } }));
        try {
            const result = await generateImage("A cute chibi anime character, white background, high quality", providerId);
            setGenTestState(prev => ({
                ...prev,
                [providerId]: { loading: false, imageUrl: result.image_url }
            }));
        } catch (e) {
            setGenTestState(prev => ({
                ...prev,
                [providerId]: { loading: false, error: String(e) }
            }));
        }
    };

    const fetchSdModels = async (providerId: string, baseUrl: string) => {
        setTestState(prev => ({ ...prev, [providerId]: { ...prev[providerId], loading: true } }));
        try {
            const models = await testSdConnection(baseUrl || "http://127.0.0.1:7860");
            setTestState(prev => ({
                ...prev,
                [providerId]: { loading: false, models }
            }));
        } catch (e) {
            setTestState(prev => ({
                ...prev,
                [providerId]: { ...prev[providerId], loading: false, error: String(e) }
            }));
        }
    };

    const updateProvider = (index: number, update: Partial<ImageGenProviderConfig>) => {
        const newProviders = [...config.providers];
        newProviders[index] = { ...newProviders[index], ...update };
        onChange({ ...config, providers: newProviders });
    };

    const addProvider = (type: string) => {
        // Generate a clean ID (e.g., "google", "google_2")
        let baseId = type.toLowerCase();
        let newId = baseId;
        let counter = 2;

        const existingIds = config.providers.map(p => p.id.toLowerCase());
        while (existingIds.includes(newId)) {
            newId = `${baseId}_${counter}`;
            counter++;
        }

        const newProvider: ImageGenProviderConfig = {
            id: newId, // The ID will be displayed in uppercase by CSS
            provider_type: type,
            enabled: true,
            api_key: "",
            model: type === "openai" ? "dall-e-3" : "",
            size: "1024x1024",
            quality: "standard",
            extra: {}
        };
        onChange({
            ...config,
            providers: [...config.providers, newProvider]
        });
        setEditingId(newId);
    };

    const removeProvider = (index: number) => {
        const newProviders = config.providers.filter((_, i) => i !== index);
        onChange({ ...config, providers: newProviders });
        if (editingId === config.providers[index].id) {
            setEditingId(null);
        }
    };

    return (
        <div className="space-y-6">
            {/* Master Toggle */}
            <div className="flex items-center justify-between p-4 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-elevated)]">
                <div>
                    <h3 className="text-sm font-heading font-bold text-[var(--color-text-primary)]">
                        {t("settings.image_gen.enable.title")}
                    </h3>
                    <p className="text-xs text-[var(--color-text-muted)]">
                        {t("settings.image_gen.enable.desc")}
                    </p>
                </div>
                <button
                    onClick={() => onChange({ ...config, enabled: !config.enabled })}
                    className={clsx(
                        "w-10 h-6 rounded-full transition-colors relative",
                        config.enabled ? "bg-emerald-500" : "bg-[var(--color-border)]"
                    )}
                >
                    <motion.div
                        animate={{ x: config.enabled ? 18 : 2 }}
                        className="absolute top-1 w-4 h-4 rounded-full bg-white shadow-sm"
                    />
                </button>
            </div>

            {/* Providers List */}
            <div className="space-y-4">
                <h3 className="text-xs font-heading font-bold text-[var(--color-text-muted)] uppercase tracking-wider">
                    {t("settings.image_gen.providers.title")}
                </h3>

                <div className="space-y-3">
                    {config.providers.map((provider, index) => (
                        <motion.div
                            key={provider.id}
                            layout
                            className={clsx(
                                "rounded-lg border overflow-hidden transition-all",
                                editingId === provider.id
                                    ? "border-[var(--color-accent)] bg-[var(--color-bg-elevated)]"
                                    : "border-[var(--color-border)] bg-black/20"
                            )}
                        >
                            {/* Header */}
                            <div className="flex items-center justify-between p-3">
                                <div className="flex items-center gap-3">
                                    <button
                                        onClick={() => updateProvider(index, { enabled: !provider.enabled })}
                                        className={clsx(
                                            "w-8 h-4 rounded-full transition-colors relative",
                                            provider.enabled ? "bg-emerald-500/50" : "bg-[var(--color-border)]"
                                        )}
                                    >
                                        <motion.div
                                            animate={{ x: provider.enabled ? 16 : 2 }}
                                            className="absolute top-0.5 w-3 h-3 rounded-full bg-white"
                                        />
                                    </button>
                                    <div className="flex flex-col">
                                        <span className={clsx(
                                            "text-sm font-heading font-bold uppercase",
                                            editingId === provider.id ? "text-[var(--color-accent)]" : "text-[var(--color-text-primary)]"
                                        )}>
                                            {provider.id}
                                        </span>
                                        <span className="text-[10px] text-[var(--color-text-muted)] uppercase">
                                            {provider.provider_type}
                                        </span>
                                    </div>
                                </div>
                                <div className="flex items-center gap-2">
                                    <button
                                        onClick={() => setEditingId(editingId === provider.id ? null : provider.id)}
                                        className="p-1.5 rounded hover:bg-white/5 text-[var(--color-text-secondary)]"
                                    >
                                        {editingId === provider.id ? t("common.actions.done") : t("common.actions.edit")}
                                    </button>
                                    <button
                                        onClick={() => removeProvider(index)}
                                        className="p-1.5 rounded hover:bg-red-500/20 text-[var(--color-text-muted)] hover:text-red-400"
                                    >
                                        <Trash2 size={14} />
                                    </button>
                                </div>
                            </div>

                            {/* Edit Form */}
                            <AnimatePresence>
                                {editingId === provider.id && (
                                    <motion.div
                                        initial={{ height: 0, opacity: 0 }}
                                        animate={{ height: "auto", opacity: 1 }}
                                        exit={{ height: 0, opacity: 0 }}
                                        className="border-t border-[var(--color-border)] p-4 space-y-4 bg-black/20"
                                    >
                                        {/* API Key (OpenAI / Google) */}
                                        {(provider.provider_type === "openai" || provider.provider_type === "google") && (
                                            <div>
                                                <label className={labelClasses}>{t("settings.image_gen.fields.api_key")}</label>
                                                <input
                                                    type="password"
                                                    value={provider.api_key || ""}
                                                    onChange={e => updateProvider(index, { api_key: e.target.value })}
                                                    placeholder={t("settings.image_gen.fields.api_key_placeholder")}
                                                    className={clsx(inputClasses, "font-mono text-xs")}
                                                />
                                            </div>
                                        )}

                                        {/* Endpoint / Base URL */}
                                        <div>
                                            <label className={labelClasses}>
                                                {provider.provider_type === "stable_diffusion"
                                                    ? t("settings.image_gen.fields.sd_url")
                                                    : t("settings.image_gen.fields.base_url")}
                                            </label>
                                            <input
                                                type="text"
                                                value={provider.base_url || ""}
                                                onChange={e => updateProvider(index, { base_url: e.target.value })}
                                                placeholder={provider.provider_type === "stable_diffusion" ? "http://127.0.0.1:7860" : "https://api.openai.com/v1"}
                                                className={clsx(inputClasses, "font-mono text-xs")}
                                            />
                                        </div>

                                        {/* Model */}
                                        <div>
                                            <div className="flex justify-between items-center mb-2">
                                                <label className={labelClasses.replace("mb-2", "mb-0")}>{t("settings.image_gen.fields.model")}</label>
                                                {provider.provider_type === "stable_diffusion" && (
                                                    <button
                                                        onClick={() => fetchSdModels(provider.id, provider.base_url || "")}
                                                        disabled={testState[provider.id]?.loading}
                                                        className="text-[10px] uppercase tracking-wider text-[var(--color-accent)] hover:underline disabled:opacity-50 flex items-center gap-1"
                                                    >
                                                        <RefreshCw size={10} className={testState[provider.id]?.loading ? "animate-spin" : ""} />
                                                        {testState[provider.id]?.loading ? t("settings.image_gen.fields.fetching") : t("settings.image_gen.fields.fetch")}
                                                    </button>
                                                )}
                                            </div>
                                            {provider.provider_type === "stable_diffusion" && (testState[provider.id]?.models?.length ?? 0) > 0 ? (
                                                <select
                                                    value={provider.model || ""}
                                                    onChange={e => updateProvider(index, { model: e.target.value })}
                                                    className={clsx(inputClasses, "font-mono text-xs appearance-none cursor-pointer")}
                                                >
                                                    <option value="">{t("settings.image_gen.fields.select_checkpoint")}</option>
                                                    {testState[provider.id].models!.map(m => (
                                                        <option key={m} value={m}>{m}</option>
                                                    ))}
                                                </select>
                                            ) : (
                                                <input
                                                    type="text"
                                                    value={provider.model || ""}
                                                    onChange={e => updateProvider(index, { model: e.target.value })}
                                                    placeholder={provider.provider_type === "openai" ? "dall-e-3" : t("settings.image_gen.fields.checkpoint_placeholder")}
                                                    className={clsx(inputClasses, "font-mono text-xs")}
                                                />
                                            )}
                                        </div>

                                        {/* Size */}
                                        <div>
                                            <label className={labelClasses}>{t("settings.image_gen.fields.size")}</label>
                                            <input
                                                type="text"
                                                value={provider.size || ""}
                                                onChange={e => updateProvider(index, { size: e.target.value })}
                                                placeholder="1024x1024"
                                                className={clsx(inputClasses, "font-mono text-xs")}
                                            />
                                        </div>

                                        {/* Test Connection (SD only) */}
                                        {provider.provider_type === "stable_diffusion" && (
                                            <div className="pt-2 border-t border-[var(--color-border)]">
                                                <button
                                                    onClick={() => handleTestSd(provider.id, provider.base_url || "")}
                                                    disabled={testState[provider.id]?.loading}
                                                    className={clsx(
                                                        "w-full py-2 text-xs rounded-lg border transition-all flex items-center justify-center gap-2",
                                                        testState[provider.id]?.result
                                                            ? "border-emerald-500/50 bg-emerald-500/10 text-emerald-400"
                                                            : testState[provider.id]?.error
                                                                ? "border-red-500/50 bg-red-500/10 text-red-400"
                                                                : "border-[var(--color-accent)] bg-[var(--color-accent)]/10 text-[var(--color-accent)] hover:bg-[var(--color-accent)]/20"
                                                    )}
                                                >
                                                    {testState[provider.id]?.loading ? (
                                                        <><RefreshCw size={12} className="animate-spin" /> {t("settings.image_gen.test.testing")}</>
                                                    ) : testState[provider.id]?.result ? (
                                                        <><CheckCircle size={12} /> {testState[provider.id].result}</>
                                                    ) : testState[provider.id]?.error ? (
                                                        <><XCircle size={12} /> {t("settings.image_gen.test.failed")}</>
                                                    ) : (
                                                        <><Wifi size={12} /> {t("settings.image_gen.test.button")}</>
                                                    )}
                                                </button>
                                                {testState[provider.id]?.error && (
                                                    <div className="mt-2 p-2 rounded bg-red-500/5 border border-red-500/10">
                                                        <p className="text-[10px] text-red-400 break-all">
                                                            {testState[provider.id].error}
                                                        </p>
                                                        <p
                                                            className="text-[10px] text-[var(--color-text-muted)] mt-1"
                                                            dangerouslySetInnerHTML={{ __html: t("settings.image_gen.test.tip") }}
                                                        />
                                                    </div>
                                                )}
                                            </div>
                                        )}

                                        {/* Test Generation */}
                                        <div className="pt-4 border-t border-[var(--color-border)]">
                                            <div className="flex items-center justify-between mb-2">
                                                <label className={clsx(labelClasses, "mb-0")}>{t("settings.image_gen.test_gen.label")}</label>
                                                <button
                                                    onClick={() => handleTestGenerate(provider.id)}
                                                    disabled={genTestState[provider.id]?.loading}
                                                    className="text-[10px] uppercase tracking-wider text-[var(--color-accent)] hover:underline disabled:opacity-50 flex items-center gap-1"
                                                >
                                                    {genTestState[provider.id]?.loading ? (
                                                        <Loader2 size={12} className="animate-spin" />
                                                    ) : (
                                                        <ImageIcon size={12} />
                                                    )}
                                                    {genTestState[provider.id]?.loading ? t("settings.image_gen.test_gen.generating") : t("settings.image_gen.test_gen.button")}
                                                </button>
                                            </div>

                                            {genTestState[provider.id]?.error && (
                                                <p className="text-[10px] text-red-400 mb-2 break-all bg-red-500/5 p-2 rounded">
                                                    {genTestState[provider.id].error}
                                                </p>
                                            )}

                                            {genTestState[provider.id]?.imageUrl && (
                                                <div className="relative aspect-square w-full rounded-lg overflow-hidden border border-[var(--color-border)] bg-black/40">
                                                    <img
                                                        src={convertFileSrc(genTestState[provider.id].imageUrl!)}
                                                        alt="Test Generation"
                                                        className="w-full h-full object-contain"
                                                    />
                                                    <div className="absolute inset-0 pointer-events-none shadow-inner" />
                                                </div>
                                            )}
                                        </div>
                                    </motion.div>
                                )}
                            </AnimatePresence>
                        </motion.div>
                    ))}
                </div>

                {/* Add Buttons */}
                <div className="grid grid-cols-2 gap-3 pt-2">
                    <button
                        onClick={() => addProvider("openai")}
                        className="flex items-center justify-center gap-2 px-4 py-3 border border-[var(--color-border)] rounded-lg hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors uppercase text-xs font-bold tracking-wider"
                    >
                        <Plus size={14} /> {t("settings.image_gen.add.openai")}
                    </button>
                    <button
                        onClick={() => addProvider("google")}
                        className="flex items-center justify-center gap-2 px-4 py-3 border border-[var(--color-border)] rounded-lg hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors uppercase text-xs font-bold tracking-wider"
                    >
                        <Plus size={14} /> {t("settings.image_gen.add.google")}
                    </button>
                    <button
                        onClick={() => addProvider("stable_diffusion")}
                        className="flex items-center justify-center gap-2 px-4 py-3 border border-[var(--color-border)] rounded-lg hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors uppercase text-xs font-bold tracking-wider"
                    >
                        <Plus size={14} /> {t("settings.image_gen.add.sd")}
                    </button>
                </div>
            </div>
        </div>
    );
}
