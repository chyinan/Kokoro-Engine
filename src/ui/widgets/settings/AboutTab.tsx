import { useEffect, useState } from "react";
import { AlertTriangle, CheckCircle2, ExternalLink, Github, Loader2, RefreshCw, Send, Tag } from "lucide-react";
import { useTranslation } from "react-i18next";
import { clsx } from "clsx";
import { getEngineInfo } from "../../../lib/kokoro-bridge";
import { sectionHeadingClasses } from "../../styles/settings-primitives";
import logoUrl from "../../../../logo.png";

const GITHUB_REPO_URL = "https://github.com/chyinan/Kokoro-Engine";
const GITHUB_RELEASES_URL = `${GITHUB_REPO_URL}/releases`;
const GITHUB_LATEST_RELEASE_API = "https://api.github.com/repos/chyinan/Kokoro-Engine/releases/latest";
const TELEGRAM_URL = "https://t.me/+U39dgiUspCo2NDNh";

type UpdateStatus = "idle" | "checking" | "latest" | "available" | "unknown" | "error";

interface GithubRelease {
    tag_name: string;
    name?: string;
    html_url: string;
    published_at?: string;
}

function normalizeVersion(value: string): string {
    return value.trim().replace(/^[^\d]*/, "").split(/[+-]/)[0];
}

function parseVersion(value: string): number[] | null {
    const normalized = normalizeVersion(value);
    if (!/^\d+(\.\d+){0,3}$/.test(normalized)) {
        return null;
    }

    return normalized.split(".").map(part => Number(part));
}

function compareVersions(a: string, b: string): number | null {
    const parsedA = parseVersion(a);
    const parsedB = parseVersion(b);
    if (!parsedA || !parsedB) {
        return null;
    }

    const length = Math.max(parsedA.length, parsedB.length);
    for (let i = 0; i < length; i += 1) {
        const left = parsedA[i] ?? 0;
        const right = parsedB[i] ?? 0;
        if (left > right) return 1;
        if (left < right) return -1;
    }

    return 0;
}

function stringifyError(error: unknown): string {
    if (typeof error === "string") return error;
    if (error instanceof Error) return error.message;
    return String(error);
}

async function openExternalUrl(url: string): Promise<void> {
    try {
        const { openUrl } = await import("@tauri-apps/plugin-opener");
        await openUrl(url);
    } catch {
        window.open(url, "_blank", "noopener,noreferrer");
    }
}

export default function AboutTab() {
    const { t, i18n } = useTranslation();
    const [currentVersion, setCurrentVersion] = useState("");
    const [release, setRelease] = useState<GithubRelease | null>(null);
    const [status, setStatus] = useState<UpdateStatus>("idle");
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        getEngineInfo()
            .then(info => setCurrentVersion(info.version))
            .catch(() => setCurrentVersion(""));
    }, []);

    const handleCheckUpdates = async () => {
        setStatus("checking");
        setError(null);
        setRelease(null);

        try {
            const response = await fetch(GITHUB_LATEST_RELEASE_API, {
                headers: {
                    Accept: "application/vnd.github+json",
                },
            });

            if (!response.ok) {
                throw new Error(`${response.status} ${response.statusText}`);
            }

            const latest = await response.json() as GithubRelease;
            if (!latest.tag_name || !latest.html_url) {
                throw new Error(t("settings.about.release_invalid"));
            }

            setRelease(latest);
            const comparison = currentVersion ? compareVersions(latest.tag_name, currentVersion) : null;
            setStatus(comparison === null ? "unknown" : comparison > 0 ? "available" : "latest");
        } catch (e) {
            setError(stringifyError(e));
            setStatus("error");
        }
    };

    const versionLabel = currentVersion || t("settings.about.version_unknown");
    const releaseVersion = release?.tag_name ?? "";
    const releaseDate = release?.published_at
        ? new Date(release.published_at).toLocaleDateString(i18n.resolvedLanguage || i18n.language, {
            year: "numeric",
            month: "short",
            day: "numeric",
        })
        : null;

    return (
        <div className="space-y-5">
            <div className="flex flex-col items-center py-3 text-center">
                <div className="mb-4 flex h-24 w-24 items-center justify-center rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-surface-soft)] p-2 shadow-sm">
                    <img
                        src={logoUrl}
                        alt={t("settings.about.logo_alt")}
                        className="h-full w-full object-contain"
                        draggable={false}
                    />
                </div>
                <h3 className="font-heading text-xl font-bold tracking-wider text-[var(--color-text-primary)]">
                    Kokoro Engine
                </h3>
                <div className="mt-1 text-xs text-[var(--color-text-muted)]">
                    {t("settings.about.current_version", { version: versionLabel })}
                </div>
            </div>

            <div>
                <div className={clsx(sectionHeadingClasses, "mb-3")}>{t("settings.about.links_title")}</div>
                <div className="grid gap-3 sm:grid-cols-2">
                    <button
                        type="button"
                        onClick={() => void openExternalUrl(GITHUB_REPO_URL)}
                        className="flex min-h-[74px] items-center gap-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-surface-soft)] px-4 py-3 text-left transition-colors hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]"
                    >
                        <Github size={20} strokeWidth={1.5} className="shrink-0" />
                        <span className="min-w-0 flex-1">
                            <span className="block text-sm font-heading font-semibold text-[var(--color-text-primary)]">
                                {t("settings.about.github")}
                            </span>
                            <span className="mt-1 block text-xs text-[var(--color-text-muted)]">
                                {t("settings.about.github_desc")}
                            </span>
                        </span>
                        <ExternalLink size={14} strokeWidth={1.5} className="shrink-0 text-[var(--color-text-muted)]" />
                    </button>

                    <button
                        type="button"
                        onClick={() => void openExternalUrl(TELEGRAM_URL)}
                        className="flex min-h-[74px] items-center gap-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-surface-soft)] px-4 py-3 text-left transition-colors hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]"
                    >
                        <Send size={20} strokeWidth={1.5} className="shrink-0" />
                        <span className="min-w-0 flex-1">
                            <span className="block text-sm font-heading font-semibold text-[var(--color-text-primary)]">
                                {t("settings.about.telegram")}
                            </span>
                            <span className="mt-1 block text-xs text-[var(--color-text-muted)]">
                                {t("settings.about.telegram_desc")}
                            </span>
                        </span>
                        <ExternalLink size={14} strokeWidth={1.5} className="shrink-0 text-[var(--color-text-muted)]" />
                    </button>
                </div>
            </div>

            <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-surface-soft)] p-4">
                <div className="mb-2 flex items-center gap-2">
                    <Tag size={16} strokeWidth={1.5} className="text-[var(--color-accent)]" />
                    <div className={sectionHeadingClasses}>{t("settings.about.updates_title")}</div>
                </div>
                <p className="mb-4 text-xs leading-relaxed text-[var(--color-text-muted)]">
                    {t("settings.about.updates_desc")}
                </p>

                <div className="flex flex-wrap items-center gap-3">
                    <button
                        type="button"
                        onClick={handleCheckUpdates}
                        disabled={status === "checking"}
                        className={clsx(
                            "inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-heading font-semibold tracking-wider uppercase transition-colors",
                            "bg-[var(--color-accent)] text-black hover:bg-white",
                            status === "checking" && "cursor-wait opacity-70"
                        )}
                    >
                        {status === "checking" ? (
                            <Loader2 size={15} strokeWidth={1.5} className="animate-spin" />
                        ) : (
                            <RefreshCw size={15} strokeWidth={1.5} />
                        )}
                        <span className="leading-none translate-y-[2px]">
                            {status === "checking" ? t("settings.about.checking") : t("settings.about.check_updates")}
                        </span>
                    </button>

                    <button
                        type="button"
                        onClick={() => void openExternalUrl(release?.html_url ?? GITHUB_RELEASES_URL)}
                        className="inline-flex items-center gap-2 rounded-md border border-[var(--color-border)] px-4 py-2 text-sm text-[var(--color-text-secondary)] transition-colors hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]"
                    >
                        <ExternalLink size={15} strokeWidth={1.5} />
                        {t("settings.about.open_releases")}
                    </button>
                </div>

                {status === "latest" && release && (
                    <div className="mt-4 flex items-start gap-2 rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-200">
                        <CheckCircle2 size={15} strokeWidth={1.5} className="mt-0.5 shrink-0" />
                        <div>
                            <div>{t("settings.about.latest", { version: releaseVersion })}</div>
                            {releaseDate && <div className="mt-1 opacity-80">{t("settings.about.published", { date: releaseDate })}</div>}
                        </div>
                    </div>
                )}

                {(status === "available" || status === "unknown") && release && (
                    <div className="mt-4 flex items-start gap-2 rounded-md border border-[var(--color-accent)]/40 bg-[var(--color-accent)]/10 px-3 py-2 text-xs text-[var(--color-text-primary)]">
                        <Tag size={15} strokeWidth={1.5} className="mt-0.5 shrink-0 text-[var(--color-accent)]" />
                        <div>
                            <div>
                                {status === "available"
                                    ? t("settings.about.available", { version: releaseVersion })
                                    : t("settings.about.latest_release", { version: releaseVersion })}
                            </div>
                            {releaseDate && <div className="mt-1 text-[var(--color-text-muted)]">{t("settings.about.published", { date: releaseDate })}</div>}
                        </div>
                    </div>
                )}

                {status === "error" && (
                    <div className="mt-4 flex items-start gap-2 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-200">
                        <AlertTriangle size={15} strokeWidth={1.5} className="mt-0.5 shrink-0" />
                        <div>{t("settings.about.error", { error })}</div>
                    </div>
                )}
            </div>
        </div>
    );
}
