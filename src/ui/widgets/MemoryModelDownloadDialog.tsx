import { AnimatePresence, motion } from "framer-motion";
import { AlertTriangle, Download, Loader2, ShieldAlert, X } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { clsx } from "clsx";
import type {
    MemoryEmbeddingModelDownloadProgress,
    MemoryEmbeddingModelStatus,
} from "../../lib/kokoro-bridge";

interface MemoryModelDownloadDialogProps {
    open: boolean;
    status: MemoryEmbeddingModelStatus | null;
    progress: MemoryEmbeddingModelDownloadProgress | null;
    downloading: boolean;
    error: string | null;
    onClose: () => void;
    onDownload: () => void;
}

function formatBytes(value: number | null | undefined): string {
    if (value == null || Number.isNaN(value)) {
        return "--";
    }

    if (value < 1024) {
        return `${value} B`;
    }

    const units = ["KB", "MB", "GB"];
    let size = value / 1024;
    let unitIndex = 0;

    while (size >= 1024 && unitIndex < units.length - 1) {
        size /= 1024;
        unitIndex += 1;
    }

    return `${size.toFixed(size >= 100 ? 0 : 1)} ${units[unitIndex]}`;
}

export default function MemoryModelDownloadDialog({
    open,
    status,
    progress,
    downloading,
    error,
    onClose,
    onDownload,
}: MemoryModelDownloadDialogProps) {
    const { t } = useTranslation();

    const progressPercent = useMemo(() => {
        if (status?.installed) {
            return 100;
        }
        if (progress?.stage === "ready") {
            return 100;
        }
        if (progress?.stage === "complete" && progress.file_count > 0) {
            return Math.max(0, Math.min(100, Math.round((progress.file_index / progress.file_count) * 100)));
        }
        if (!progress?.total_bytes || progress.total_bytes <= 0) {
            if (progress?.stage === "downloading" && progress.file_count > 0) {
                const completedFiles = Math.max(0, progress.file_index - 1);
                const currentFileShare = progress.downloaded_bytes > 0 ? 0.5 : 0;
                return Math.max(0, Math.min(99, Math.round(((completedFiles + currentFileShare) / progress.file_count) * 100)));
            }
            return null;
        }
        const currentFilePercent = progress.downloaded_bytes / progress.total_bytes;
        const completedFiles = Math.max(0, progress.file_index - 1);
        const aggregatePercent = progress.file_count > 0
            ? (completedFiles + currentFilePercent) / progress.file_count
            : currentFilePercent;
        return Math.max(0, Math.min(100, Math.round(aggregatePercent * 100)));
    }, [progress, status?.installed]);

    const stageLabel = status?.installed
        ? t("onboarding.memory_model.status.ready")
        : progress?.stage === "checking"
            ? t("onboarding.memory_model.status.checking")
            : progress?.stage === "verifying"
                ? t("onboarding.memory_model.status.verifying")
                : progress?.stage === "complete"
                    ? t("onboarding.memory_model.status.complete")
                    : progress?.stage === "downloading"
                        ? t("onboarding.memory_model.status.downloading")
                        : error
                            ? t("onboarding.memory_model.status.error")
                            : t("onboarding.memory_model.status.pending");

    const primaryLabel = status?.installed
        ? t("onboarding.memory_model.actions.continue")
        : downloading
            ? t("onboarding.memory_model.actions.downloading")
            : error
                ? t("onboarding.memory_model.actions.retry")
                : t("onboarding.memory_model.actions.download");

    return (
        <AnimatePresence>
            {open && (
                <motion.div
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                    className="fixed inset-0 z-[170] flex items-center justify-center bg-black/65 px-4 backdrop-blur-sm"
                >
                    <motion.div
                        initial={{ opacity: 0, y: 20, scale: 0.98 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: 12, scale: 0.98 }}
                        transition={{ type: "spring", stiffness: 280, damping: 28 }}
                        className="w-full max-w-[520px] rounded-3xl border border-[var(--color-border-accent)] bg-[var(--color-bg-elevated)]/95 shadow-2xl backdrop-blur-2xl"
                    >
                        <div className="flex items-start justify-between gap-4 border-b border-[var(--color-border)] px-6 py-5">
                            <div className="space-y-2">
                                <div className="flex items-center gap-2 text-[11px] font-heading font-semibold uppercase tracking-[0.18em] text-[var(--color-accent)]">
                                    <Download size={14} strokeWidth={1.6} />
                                    {t("onboarding.memory_model.eyebrow")}
                                </div>
                                <h2 className="font-heading text-xl font-bold tracking-wide text-[var(--color-text-primary)]">
                                    {t("onboarding.memory_model.title")}
                                </h2>
                            </div>
                            <button
                                type="button"
                                onClick={onClose}
                                className="rounded-lg p-2 text-[var(--color-text-muted)] transition-colors hover:text-[var(--color-accent)]"
                                aria-label={t("onboarding.memory_model.actions.close")}
                                title={t("onboarding.memory_model.actions.close")}
                            >
                                <X size={16} strokeWidth={1.7} />
                            </button>
                        </div>

                        <div className="space-y-4 px-6 py-5">
                            <p className="text-sm leading-6 text-[var(--color-text-secondary)]">
                                {t("onboarding.memory_model.description")}
                            </p>

                            <div className="rounded-2xl border border-amber-400/30 bg-amber-500/10 p-4">
                                <div className="mb-2 flex items-center gap-2 text-sm font-heading font-semibold tracking-wide text-amber-200">
                                    <ShieldAlert size={16} strokeWidth={1.7} />
                                    {t("onboarding.memory_model.warning_title")}
                                </div>
                                <p className="text-sm leading-6 text-amber-100/90">
                                    {t("onboarding.memory_model.warning")}
                                </p>
                            </div>

                            <div className="rounded-2xl border border-[var(--color-border)] bg-black/20 p-4">
                                <div className="mb-3 flex items-center justify-between gap-3">
                                    <div>
                                        <div className="text-xs font-heading font-semibold uppercase tracking-[0.14em] text-[var(--color-text-muted)]">
                                            {t("onboarding.memory_model.progress_label")}
                                        </div>
                                        <div className="mt-1 text-sm font-medium text-[var(--color-text-primary)]">
                                            {stageLabel}
                                        </div>
                                    </div>
                                    {downloading && (
                                        <Loader2 size={16} strokeWidth={1.7} className="animate-spin text-[var(--color-accent)]" />
                                    )}
                                </div>

                                <div className="h-3 overflow-hidden rounded-full bg-white/10">
                                    <motion.div
                                        initial={false}
                                        animate={{
                                            width: progressPercent == null
                                                ? (downloading ? "35%" : status?.installed ? "100%" : "0%")
                                                : `${progressPercent}%`,
                                        }}
                                        transition={{
                                            duration: progressPercent == null ? 0.8 : 0.3,
                                            repeat: progressPercent == null && downloading ? Infinity : 0,
                                            repeatType: "reverse",
                                        }}
                                        className={clsx(
                                            "h-full rounded-full",
                                            status?.installed
                                                ? "bg-emerald-400"
                                                : "bg-[var(--color-accent)]"
                                        )}
                                    />
                                </div>

                                <div className="mt-3 flex items-center justify-between gap-3 text-xs text-[var(--color-text-muted)]">
                                    <span>
                                        {progress?.current_file
                                            ? t("onboarding.memory_model.current_file", {
                                                file: progress.current_file,
                                                index: progress.file_index,
                                                total: Math.max(progress.file_count, 1),
                                            })
                                            : t("onboarding.memory_model.waiting")}
                                    </span>
                                    <span>
                                        {progress?.total_bytes
                                            ? `${formatBytes(progress.downloaded_bytes)} / ${formatBytes(progress.total_bytes)}`
                                            : progress?.downloaded_bytes
                                                ? formatBytes(progress.downloaded_bytes)
                                                : "--"}
                                    </span>
                                </div>
                            </div>

                            {status?.install_dir && (
                                <div className="rounded-2xl border border-[var(--color-border)] bg-black/15 px-4 py-3 text-xs leading-5 text-[var(--color-text-muted)]">
                                    {t("onboarding.memory_model.install_path", { path: status.install_dir })}
                                </div>
                            )}

                            {error && (
                                <div className="rounded-2xl border border-red-500/40 bg-red-500/10 p-4 text-sm leading-6 text-red-200">
                                    <div className="mb-1 flex items-center gap-2 font-heading font-semibold tracking-wide">
                                        <AlertTriangle size={15} strokeWidth={1.7} />
                                        {t("onboarding.memory_model.error_title")}
                                    </div>
                                    <div>{error}</div>
                                </div>
                            )}
                        </div>

                        <div className="flex items-center justify-between gap-3 border-t border-[var(--color-border)] px-6 py-5">
                            <button
                                type="button"
                                onClick={onClose}
                                className="inline-flex h-10 items-center justify-center rounded-lg border border-[var(--color-border)] px-4 text-sm font-heading font-semibold uppercase tracking-[0.14em] text-[var(--color-text-secondary)] transition-colors hover:border-[var(--color-border-accent)] hover:text-[var(--color-accent)]"
                            >
                                {t("onboarding.memory_model.actions.close")}
                            </button>
                            <button
                                type="button"
                                onClick={onDownload}
                                disabled={downloading}
                                className={clsx(
                                    "inline-flex h-10 items-center justify-center rounded-lg px-4 text-sm font-heading font-semibold uppercase tracking-[0.14em] transition-colors",
                                    downloading
                                        ? "cursor-not-allowed bg-white/10 text-[var(--color-text-muted)]"
                                        : "bg-[var(--color-accent)] text-black hover:bg-white"
                                )}
                            >
                                {primaryLabel}
                            </button>
                        </div>
                    </motion.div>
                </motion.div>
            )}
        </AnimatePresence>
    );
}
