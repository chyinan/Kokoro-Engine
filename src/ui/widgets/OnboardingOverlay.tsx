import { useEffect, useMemo, useState, type CSSProperties } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { clsx } from "clsx";
import { Languages, MousePointerClick, Sparkles, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { SettingsTabId } from "./SettingsPanel";

export type OnboardingStep = "language" | "open-settings" | "api" | "persona" | "return-home" | "chat";
export type OnboardingLanguageCode = "en" | "zh" | "ja" | "ko" | "ru";

interface OnboardingOverlayProps {
    step: OnboardingStep | null;
    selectedLanguage: OnboardingLanguageCode;
    settingsOpen: boolean;
    activeSettingsTab: SettingsTabId;
    onLanguageSelect: (language: OnboardingLanguageCode) => void;
    onAdvance: () => void;
    onDismiss: () => void;
}

interface SpotlightRect {
    top: number;
    left: number;
    width: number;
    height: number;
}

interface StepMeta {
    targetIds: string[];
    title: string;
    description: string;
    actionLabel: string;
    canAdvance: boolean;
    helperText?: string;
}

const LANGUAGE_OPTIONS: Array<{ code: OnboardingLanguageCode; label: string }> = [
    { code: "zh", label: "中文" },
    { code: "en", label: "English" },
    { code: "ja", label: "日本語" },
    { code: "ko", label: "한국어" },
    { code: "ru", label: "Русский" },
];

const STEP_ORDER: Record<OnboardingStep, number> = {
    language: 1,
    "open-settings": 2,
    api: 3,
    persona: 4,
    "return-home": 5,
    chat: 6,
};

function clamp(value: number, min: number, max: number) {
    return Math.min(Math.max(value, min), max);
}

function hasTarget(targetId: string) {
    if (typeof document === "undefined") {
        return false;
    }

    return Boolean(document.querySelector(`[data-onboarding-id="${targetId}"]`));
}

function findTargetElement(targetIds: string[]) {
    if (typeof document === "undefined") {
        return null;
    }

    for (const targetId of targetIds) {
        const element = document.querySelector<HTMLElement>(`[data-onboarding-id="${targetId}"]`);
        if (element) {
            return element;
        }
    }

    return null;
}

function rectEquals(a: SpotlightRect | null, b: SpotlightRect | null) {
    if (a === b) {
        return true;
    }

    if (!a || !b) {
        return false;
    }

    return a.top === b.top && a.left === b.left && a.width === b.width && a.height === b.height;
}

export default function OnboardingOverlay({
    step,
    selectedLanguage,
    settingsOpen,
    activeSettingsTab,
    onLanguageSelect,
    onAdvance,
    onDismiss,
}: OnboardingOverlayProps) {
    const { t } = useTranslation();
    const [targetRect, setTargetRect] = useState<SpotlightRect | null>(null);

    const stepMeta = useMemo<StepMeta | null>(() => {
        if (!step) {
            return null;
        }

        if (step === "language") {
            return {
                targetIds: [],
                title: t("onboarding.language.title"),
                description: t("onboarding.language.description"),
                actionLabel: t("onboarding.actions.start"),
                canAdvance: true,
            };
        }

        if (step === "open-settings") {
            return {
                targetIds: ["settings-button"],
                title: t("onboarding.steps.open_settings.title"),
                description: t("onboarding.steps.open_settings.description"),
                actionLabel: t("onboarding.actions.next"),
                canAdvance: false,
                helperText: t("onboarding.hints.click_highlighted"),
            };
        }

        if (!settingsOpen && (step === "api" || step === "persona")) {
            return {
                targetIds: ["settings-button"],
                title: t("onboarding.steps.open_settings.title"),
                description: t("onboarding.steps.open_settings.description"),
                actionLabel: t("onboarding.actions.next"),
                canAdvance: false,
                helperText: t("onboarding.hints.click_highlighted"),
            };
        }

        if (step === "api") {
            const opened = activeSettingsTab === "api";
            return {
                targetIds: ["settings-tab-api"],
                title: opened ? t("onboarding.steps.api.done_title") : t("onboarding.steps.api.title"),
                description: opened ? t("onboarding.steps.api.done_description") : t("onboarding.steps.api.description"),
                actionLabel: t("onboarding.actions.next"),
                canAdvance: opened,
                helperText: opened ? undefined : t("onboarding.hints.click_highlighted"),
            };
        }

        if (step === "persona") {
            const opened = activeSettingsTab === "persona";
            return {
                targetIds: ["settings-tab-persona"],
                title: opened ? t("onboarding.steps.persona.done_title") : t("onboarding.steps.persona.title"),
                description: opened ? t("onboarding.steps.persona.done_description") : t("onboarding.steps.persona.description"),
                actionLabel: t("onboarding.actions.next"),
                canAdvance: opened,
                helperText: opened ? undefined : t("onboarding.hints.click_highlighted"),
            };
        }

        if (step === "return-home") {
            return {
                targetIds: ["settings-close-button", "settings-cancel-button"],
                title: t("onboarding.steps.return_home.title"),
                description: t("onboarding.steps.return_home.description"),
                actionLabel: t("onboarding.actions.next"),
                canAdvance: false,
                helperText: t("onboarding.hints.click_highlighted"),
            };
        }

        const chatExpanded = hasTarget("chat-input");
        return {
            targetIds: chatExpanded ? ["chat-input"] : ["chat-open-button"],
            title: chatExpanded ? t("onboarding.steps.chat.title") : t("onboarding.steps.chat.collapsed_title"),
            description: chatExpanded ? t("onboarding.steps.chat.description") : t("onboarding.steps.chat.collapsed_description"),
            actionLabel: t("onboarding.actions.finish"),
            canAdvance: chatExpanded,
            helperText: chatExpanded ? undefined : t("onboarding.hints.click_highlighted"),
        };
    }, [activeSettingsTab, settingsOpen, step, t]);

    useEffect(() => {
        if (!stepMeta || stepMeta.targetIds.length === 0) {
            setTargetRect(null);
            return;
        }

        const updateRect = () => {
            const element = findTargetElement(stepMeta.targetIds);
            if (!element) {
                setTargetRect((prev) => (prev === null ? prev : null));
                return;
            }

            const bounds = element.getBoundingClientRect();
            const nextRect = {
                top: Math.round(bounds.top),
                left: Math.round(bounds.left),
                width: Math.round(bounds.width),
                height: Math.round(bounds.height),
            };

            setTargetRect((prev) => (rectEquals(prev, nextRect) ? prev : nextRect));
        };

        updateRect();
        window.addEventListener("resize", updateRect);
        window.addEventListener("scroll", updateRect, true);
        let frameId = 0;
        const tick = () => {
            updateRect();
            frameId = window.requestAnimationFrame(tick);
        };
        frameId = window.requestAnimationFrame(tick);

        return () => {
            window.removeEventListener("resize", updateRect);
            window.removeEventListener("scroll", updateRect, true);
            window.cancelAnimationFrame(frameId);
        };
    }, [stepMeta]);

    if (!step || !stepMeta) {
        return null;
    }

    const viewportWidth = typeof window === "undefined" ? 1280 : window.innerWidth;
    const viewportHeight = typeof window === "undefined" ? 720 : window.innerHeight;
    const cardWidth = Math.min(360, Math.max(280, viewportWidth - 32));
    const estimatedCardHeight = step === "language" ? 280 : 210;
    let cardLeft = Math.round((viewportWidth - cardWidth) / 2);
    let cardTop = Math.round((viewportHeight - estimatedCardHeight) / 2);

    if (targetRect) {
        const margin = 16;
        cardLeft = Math.round(
            clamp(
                targetRect.left + targetRect.width / 2 - cardWidth / 2,
                margin,
                viewportWidth - cardWidth - margin,
            )
        );

        const spaceBelow = viewportHeight - (targetRect.top + targetRect.height);
        const placeBelow = spaceBelow >= estimatedCardHeight + 32 || spaceBelow >= targetRect.top;
        cardTop = placeBelow
            ? Math.round(clamp(targetRect.top + targetRect.height + 16, margin, viewportHeight - estimatedCardHeight - margin))
            : Math.round(clamp(targetRect.top - estimatedCardHeight - 16, margin, viewportHeight - estimatedCardHeight - margin));
    }

    const cardStyle: CSSProperties = {
        width: cardWidth,
        left: cardLeft,
        top: cardTop,
    };

    return (
        <AnimatePresence>
            <motion.div
                key={step}
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                className="fixed inset-0 z-[140] pointer-events-none"
            >
                {!targetRect && (
                    <div className="absolute inset-0 bg-slate-950/72" />
                )}

                {targetRect && (
                    <div
                        className="absolute rounded-2xl border border-[var(--color-border-accent)] bg-[var(--color-accent)]/6"
                        style={{
                            top: targetRect.top - 8,
                            left: targetRect.left - 8,
                            width: targetRect.width + 16,
                            height: targetRect.height + 16,
                            boxShadow: "0 0 0 9999px rgba(2, 6, 23, 0.72), 0 0 24px rgba(0, 240, 255, 0.35)",
                        }}
                    />
                )}

                <motion.div
                    initial={{ opacity: 0, y: 16, scale: 0.97 }}
                    animate={{ opacity: 1, y: 0, scale: 1 }}
                    exit={{ opacity: 0, y: 12, scale: 0.97 }}
                    transition={{ type: "spring", stiffness: 280, damping: 28 }}
                    className="absolute pointer-events-auto rounded-2xl border border-[var(--color-border-accent)] bg-[var(--color-bg-elevated)]/95 shadow-2xl backdrop-blur-xl"
                    style={cardStyle}
                >
                    <div className="flex items-start justify-between gap-4 border-b border-[var(--color-border)] px-5 py-4">
                        <div>
                            <div className="mb-2 flex items-center gap-2 text-[11px] font-heading font-semibold uppercase tracking-[0.18em] text-[var(--color-accent)]">
                                <Sparkles size={14} strokeWidth={1.6} />
                                {step === "language" ? t("onboarding.language.eyebrow") : t("onboarding.title")}
                                <span className="text-[var(--color-text-muted)]">
                                    {STEP_ORDER[step]}/6
                                </span>
                            </div>
                            <h2 className="font-heading text-lg font-bold tracking-wide text-[var(--color-text-primary)]">
                                {stepMeta.title}
                            </h2>
                        </div>
                        <button
                            type="button"
                            onClick={onDismiss}
                            className="rounded-lg p-2 text-[var(--color-text-muted)] transition-colors hover:text-[var(--color-accent)]"
                            aria-label={t("onboarding.dismiss")}
                            title={t("onboarding.dismiss")}
                        >
                            <X size={16} strokeWidth={1.7} />
                        </button>
                    </div>

                    <div className="space-y-4 px-5 py-4">
                        <p className="text-sm leading-6 text-[var(--color-text-secondary)]">
                            {stepMeta.description}
                        </p>

                        {stepMeta.helperText && (
                            <div className="relative h-10 rounded-xl border border-[var(--color-border-accent)] bg-[var(--color-accent)]/10 px-10 text-xs text-[var(--color-accent)]">
                                <MousePointerClick
                                    size={14}
                                    strokeWidth={1.6}
                                    className="absolute left-3.5 top-1/2 -translate-y-1/2"
                                />
                                <span className="absolute inset-0 flex items-center justify-center px-10 text-center font-heading font-semibold tracking-[0.08em] leading-none translate-y-px">
                                    {stepMeta.helperText}
                                </span>
                            </div>
                        )}

                        {step === "language" && (
                            <div className="space-y-4">
                                <div className="flex items-center gap-2 text-xs font-heading font-semibold uppercase tracking-[0.18em] text-[var(--color-text-muted)]">
                                    <Languages size={14} strokeWidth={1.5} />
                                    {t("settings.app_language.label")}
                                </div>
                                <div className="flex flex-wrap gap-2">
                                    {LANGUAGE_OPTIONS.map((option) => (
                                        <button
                                            key={option.code}
                                            type="button"
                                            onClick={() => onLanguageSelect(option.code)}
                                            className={clsx(
                                                "rounded-xl border px-3 py-2 text-sm transition-all",
                                                selectedLanguage === option.code
                                                    ? "border-[var(--color-border-accent)] bg-[var(--color-accent)]/15 text-[var(--color-accent)] shadow-[var(--glow-accent)]"
                                                    : "border-[var(--color-border)] bg-black/20 text-[var(--color-text-secondary)] hover:border-[var(--color-border-accent)] hover:text-[var(--color-text-primary)]"
                                            )}
                                        >
                                            {option.label}
                                        </button>
                                    ))}
                                </div>
                            </div>
                        )}
                    </div>

                    <div className="flex items-center justify-between gap-3 border-t border-[var(--color-border)] px-5 py-4">
                        <button
                            type="button"
                            onClick={onDismiss}
                            className="inline-flex h-10 items-center justify-center rounded-lg border border-[var(--color-border)] px-4 text-sm leading-none font-heading font-semibold uppercase tracking-[0.14em] text-[var(--color-text-secondary)] transition-colors hover:border-[var(--color-border-accent)] hover:text-[var(--color-accent)]"
                        >
                            {t("onboarding.dismiss")}
                        </button>
                        <button
                            type="button"
                            onClick={onAdvance}
                            disabled={!stepMeta.canAdvance}
                            className={clsx(
                                "inline-flex h-10 items-center justify-center rounded-lg px-4 text-sm leading-none font-heading font-semibold uppercase tracking-[0.14em] transition-colors",
                                stepMeta.canAdvance
                                    ? "bg-[var(--color-accent)] text-black hover:bg-white"
                                    : "cursor-not-allowed bg-white/10 text-[var(--color-text-muted)]"
                            )}
                        >
                            {stepMeta.actionLabel}
                        </button>
                    </div>
                </motion.div>
            </motion.div>
        </AnimatePresence>
    );
}
