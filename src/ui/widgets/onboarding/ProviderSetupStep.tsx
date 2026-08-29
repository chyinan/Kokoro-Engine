// pattern: Imperative Shell

import { useMemo } from "react";
import { Check, RefreshCw, Save, Wifi } from "lucide-react";
import { useTranslation } from "react-i18next";
import { clsx } from "clsx";
import {
    OPENAI_COMPATIBLE_PRESETS,
    normalizeProviderSetup,
    type ProviderSetup,
    type SupportedProviderType,
} from "../../../features/onboarding/provider-setup";
import type { LlmConnectionTestResult } from "../../../lib/kokoro-bridge";
import { inputClasses, labelClasses } from "../../styles/settings-primitives";

export type ProviderSetupStepProps = Readonly<{
    setup: ProviderSetup;
    onChange: (setup: ProviderSetup) => void;
    discoveredModels?: ReadonlyArray<string>;
    isDiscovering?: boolean;
    isTesting?: boolean;
    isSaving?: boolean;
    connectionResult?: LlmConnectionTestResult | null;
    error?: string | null;
    onDiscoverModels?: () => Promise<void> | void;
    onTestConnection?: () => Promise<void> | void;
    onSave?: () => Promise<void> | void;
}>;

function requiresApiKey(setup: Readonly<ProviderSetup>): boolean {
    if (setup.providerType === "ollama" || setup.providerType === "llama_cpp") return false;
    const preset = OPENAI_COMPATIBLE_PRESETS.find((candidate) => candidate.id === setup.presetId);
    return preset?.requiresApiKey ?? true;
}

/** Compact provider setup used by onboarding; advanced generation controls stay in Settings. */
export function ProviderSetupStep({
    setup,
    onChange,
    discoveredModels = [],
    isDiscovering = false,
    isTesting = false,
    isSaving = false,
    connectionResult = null,
    error = null,
    onDiscoverModels,
    onTestConnection,
    onSave,
}: ProviderSetupStepProps) {
    const { t } = useTranslation();
    const normalized = useMemo(() => normalizeProviderSetup(setup), [setup]);
    const keyRequired = requiresApiKey(normalized);
    const presetOptions = useMemo(
        () => [
            { id: "", label: t("onboarding.workflow.provider.custom", { defaultValue: "Custom endpoint" }) },
            ...OPENAI_COMPATIBLE_PRESETS.map((preset) => ({ id: preset.id, label: preset.label })),
        ],
        [t],
    );

    const update = (updates: Partial<ProviderSetup>): void => {
        onChange(normalizeProviderSetup({ ...normalized, ...updates }));
    };

    const handlePresetChange = (presetId: string): void => {
        const selected = OPENAI_COMPATIBLE_PRESETS.find((preset) => preset.id === presetId);
        if (!selected) {
            update({ presetId: null });
            return;
        }
        update({
            presetId: selected.id,
            providerType: selected.providerType,
            endpoint: selected.baseUrl,
            model: selected.model,
        });
    };

    const handleProviderTypeChange = (providerType: SupportedProviderType): void => {
        update({ providerType, presetId: null, endpoint: "", model: "", apiKey: null });
    };

    return (
        <div className="space-y-4" data-onboarding-id="provider-setup">
            <div>
                <label className={labelClasses} htmlFor="onboarding-provider-type">
                    {t("onboarding.workflow.provider.provider_type", { defaultValue: "Provider" })}
                </label>
                <select
                    id="onboarding-provider-type"
                    value={normalized.providerType}
                    onChange={(event) => handleProviderTypeChange(event.target.value as SupportedProviderType)}
                    className={clsx(inputClasses, "font-mono")}
                >
                    <option value="openai">{t("onboarding.workflow.provider.openai", { defaultValue: "OpenAI-Compatible" })}</option>
                    <option value="ollama">{t("onboarding.workflow.provider.ollama", { defaultValue: "Ollama (local)" })}</option>
                </select>
            </div>

            {normalized.providerType === "openai" && (
                <div>
                    <label className={labelClasses} htmlFor="onboarding-provider-preset">
                        {t("onboarding.workflow.provider.preset", { defaultValue: "Preset" })}
                    </label>
                    <select
                        id="onboarding-provider-preset"
                        value={normalized.presetId ?? ""}
                        onChange={(event) => handlePresetChange(event.target.value)}
                        className={clsx(inputClasses, "font-mono")}
                    >
                        {presetOptions.map((preset) => <option key={preset.id} value={preset.id}>{preset.label}</option>)}
                    </select>
                </div>
            )}

            <div>
                <label className={labelClasses} htmlFor="onboarding-provider-endpoint">
                    {t("onboarding.workflow.provider.endpoint", { defaultValue: "Endpoint" })}
                </label>
                <input
                    id="onboarding-provider-endpoint"
                    type="url"
                    value={normalized.endpoint}
                    onChange={(event) => update({ endpoint: event.target.value, presetId: null })}
                    className={clsx(inputClasses, "font-mono")}
                    placeholder={normalized.providerType === "ollama" ? "http://localhost:11434" : "https://api.openai.com/v1"}
                    required
                />
            </div>

            {keyRequired && (
                <div>
                    <label className={labelClasses} htmlFor="onboarding-provider-key">
                        {t("onboarding.workflow.provider.api_key", { defaultValue: "API key" })}
                    </label>
                    <input
                        id="onboarding-provider-key"
                        type="password"
                        value={normalized.apiKey ?? ""}
                        onChange={(event) => update({ apiKey: event.target.value })}
                        className={clsx(inputClasses, "font-mono")}
                        autoComplete="off"
                        required
                    />
                </div>
            )}

            <div>
                <div className="flex items-center justify-between gap-2">
                    <label className={labelClasses} htmlFor="onboarding-provider-model">
                        {t("onboarding.workflow.provider.model", { defaultValue: "Model" })}
                    </label>
                    <button
                        type="button"
                        onClick={onDiscoverModels}
                        disabled={!onDiscoverModels || isDiscovering || (keyRequired && !normalized.apiKey)}
                        className="inline-flex items-center gap-1 text-[10px] uppercase tracking-wider text-[var(--color-accent)] hover:underline disabled:opacity-50"
                    >
                        <RefreshCw size={11} className={isDiscovering ? "animate-spin" : ""} />
                        {isDiscovering
                            ? t("onboarding.workflow.provider.discovering", { defaultValue: "Discovering" })
                            : t("onboarding.workflow.provider.discover", { defaultValue: "Discover models" })}
                    </button>
                </div>
                <input
                    id="onboarding-provider-model"
                    type="text"
                    value={normalized.model}
                    onChange={(event) => update({ model: event.target.value })}
                    className={clsx(inputClasses, "font-mono")}
                    list="onboarding-provider-model-list"
                    required
                />
                <datalist id="onboarding-provider-model-list">
                    {discoveredModels.map((model) => <option key={model} value={model} />)}
                </datalist>
            </div>

            {error && <p role="alert" className="rounded-lg bg-red-400/10 px-3 py-2 text-xs text-red-300">{error}</p>}
            {connectionResult && (
                <p className="flex items-center gap-2 rounded-lg bg-emerald-400/10 px-3 py-2 text-xs text-emerald-300" role="status">
                    <Check size={13} />
                    {t("onboarding.workflow.provider.test_success", {
                        defaultValue: "Connection test passed",
                        count: connectionResult.tested_targets.length,
                    })}
                </p>
            )}

            <div className="grid grid-cols-2 gap-2 border-t border-[var(--color-border)] pt-3">
                <button
                    type="button"
                    data-onboarding-action="test-connection"
                    onClick={onTestConnection}
                    disabled={!onTestConnection || isTesting || isSaving || (keyRequired && !normalized.apiKey)}
                    className="inline-flex items-center justify-center gap-1.5 rounded-lg border border-[var(--color-border)] px-3 py-2 text-xs text-[var(--color-text-main)] transition-colors hover:border-[var(--color-accent)] disabled:opacity-50"
                >
                    <Wifi size={12} />
                    {isTesting ? t("onboarding.workflow.provider.testing", { defaultValue: "Testing" }) : t("onboarding.workflow.provider.test", { defaultValue: "Test connection" })}
                </button>
                <button
                    type="button"
                    data-onboarding-action="save-provider"
                    onClick={onSave}
                    disabled={!onSave || isSaving || isTesting || !normalized.endpoint || !normalized.model || (keyRequired && !normalized.apiKey)}
                    className="inline-flex items-center justify-center gap-1.5 rounded-lg border border-[var(--color-accent)] bg-[var(--color-accent)]/10 px-3 py-2 text-xs text-[var(--color-accent)] transition-colors hover:bg-[var(--color-accent)]/20 disabled:opacity-50"
                >
                    <Save size={12} />
                    {isSaving ? t("onboarding.workflow.provider.saving", { defaultValue: "Saving" }) : t("onboarding.workflow.provider.save", { defaultValue: "Save provider" })}
                </button>
            </div>
        </div>
    );
}
