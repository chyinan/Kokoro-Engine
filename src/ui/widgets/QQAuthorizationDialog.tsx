// pattern: Functional Core

import { AnimatePresence, motion } from "framer-motion";
import { Check, Loader2, ShieldCheck, UserRound, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { QQAuthorizationRequest } from "../../lib/kokoro-bridge";

type QQAuthorizationDialogProps = {
    request: QQAuthorizationRequest | null;
    busy: boolean;
    error: string | null;
    onAllow: () => void;
    onReject: () => void;
};

export function QQAuthorizationDialog({
    request,
    busy,
    error,
    onAllow,
    onReject,
}: QQAuthorizationDialogProps) {
    const { t } = useTranslation();
    const isGroup = request?.conversation_kind === "group";

    return (
        <AnimatePresence>
            {request && (
                <motion.div
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                    className="fixed inset-0 z-[180] flex items-center justify-center bg-black/65 px-4 backdrop-blur-sm"
                >
                    <motion.div
                        initial={{ opacity: 0, y: 18, scale: 0.98 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: 10, scale: 0.98 }}
                        role="dialog"
                        aria-modal="true"
                        aria-labelledby="qq-authorization-title"
                        className="w-full max-w-md rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-6 shadow-2xl"
                    >
                        <div className="flex items-start gap-4">
                            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-cyan-500/15 text-cyan-300">
                                <ShieldCheck size={22} />
                            </div>
                            <div className="min-w-0 flex-1">
                                <h2 id="qq-authorization-title" className="text-base font-semibold text-[var(--color-text-primary)]">
                                    {t(isGroup ? "bot.qq.authorization.group_title" : "bot.qq.authorization.title")}
                                </h2>
                                <p className="mt-2 text-sm leading-6 text-[var(--color-text-secondary)]">
                                    {t(isGroup ? "bot.qq.authorization.group_description" : "bot.qq.authorization.description")}
                                </p>
                            </div>
                        </div>

                        {request.group_openid && (
                            <div className="mt-5 flex items-center gap-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-3 py-3">
                                <ShieldCheck size={17} className="shrink-0 text-[var(--color-text-muted)]" />
                                <div className="min-w-0">
                                    <div className="text-xs text-[var(--color-text-muted)]">
                                        {t("bot.qq.authorization.group_openid")}
                                    </div>
                                    <code className="mt-1 block break-all text-sm text-[var(--color-text-primary)]">
                                        {request.group_openid}
                                    </code>
                                </div>
                            </div>
                        )}

                        <div className={request.group_openid ? "mt-3 flex items-center gap-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-3 py-3" : "mt-5 flex items-center gap-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-3 py-3"}>
                            <UserRound size={17} className="shrink-0 text-[var(--color-text-muted)]" />
                            <div className="min-w-0">
                                <div className="text-xs text-[var(--color-text-muted)]">
                                    {t("bot.qq.authorization.openid")}
                                </div>
                                <code className="mt-1 block break-all text-sm text-[var(--color-text-primary)]">
                                    {request.user_openid}
                                </code>
                            </div>
                        </div>

                        <p className="mt-3 text-xs leading-5 text-[var(--color-text-muted)]">
                            {t(isGroup ? "bot.qq.authorization.group_security_note" : "bot.qq.authorization.security_note")}
                        </p>

                        {error && (
                            <div className="mt-4 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-300">
                                {error}
                            </div>
                        )}

                        <div className="mt-6 flex justify-end gap-3">
                            <button
                                type="button"
                                disabled={busy}
                                onClick={onReject}
                                className="inline-flex h-9 items-center gap-2 rounded-lg border border-[var(--color-border)] px-4 text-sm text-[var(--color-text-secondary)] transition-colors hover:bg-white/5 disabled:opacity-50"
                            >
                                <X size={16} />
                                {t("bot.qq.authorization.reject")}
                            </button>
                            <button
                                type="button"
                                disabled={busy}
                                onClick={onAllow}
                                className="inline-flex h-9 items-center gap-2 rounded-lg bg-cyan-400 px-4 text-sm font-medium text-slate-950 transition-colors hover:bg-cyan-300 disabled:opacity-50"
                            >
                                {busy ? <Loader2 size={16} className="animate-spin" /> : <Check size={16} />}
                                {t("bot.qq.authorization.allow")}
                            </button>
                        </div>
                    </motion.div>
                </motion.div>
            )}
        </AnimatePresence>
    );
}
