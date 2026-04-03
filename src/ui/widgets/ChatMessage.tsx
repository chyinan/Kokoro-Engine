import { useState, memo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { Edit2, RefreshCw, Check, X, Languages, CornerDownLeft, ChevronDown, Wrench } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ToolTraceItem } from "../../lib/kokoro-bridge";

interface ChatMessageProps {
    message: {
        role: "user" | "kokoro" | "tool";
        text: string;
        images?: string[];
        translation?: string;
        isError?: boolean;
        tools?: ToolTraceItem[];
    };
    index: number;
    isStreaming: boolean;
    isTranslationExpanded: boolean;
    onToggleTranslation: () => void;
    onEdit: (newText: string) => void;
    onRegenerate: () => void;
    onContinueFrom: () => void;
    onApproveTool: (tool: ToolTraceItem) => void;
    onRejectTool: (tool: ToolTraceItem) => void;
}

function canResolvePendingTool(tool: ToolTraceItem): boolean {
    return tool.denyKind === "pending_approval"
        && tool.approvalStatus === "requested"
        && typeof tool.approvalRequestId === "string"
        && tool.approvalRequestId.length > 0;
}

type TranslateFn = (key: string, options?: Record<string, unknown>) => string;

function getToolStatusLabel(tool: ToolTraceItem, t: TranslateFn): string {
    if (tool.approvalStatus === "approved") {
        return t("chat.tools.status.approved", { defaultValue: "Approved" });
    }
    if (tool.approvalStatus === "rejected") {
        return t("chat.tools.status.rejected", { defaultValue: "Rejected" });
    }
    if (tool.denyKind) {
        const entry = toolStatusLabel[tool.denyKind];
        return t(entry.key, { defaultValue: entry.defaultValue });
    }
    return t("chat.tools.status.allowed", { defaultValue: "Allowed" });
}

function getToolStatusClassName(tool: ToolTraceItem): string {
    if (tool.approvalStatus === "approved") {
        return "bg-emerald-500/15 text-emerald-300";
    }
    if (tool.approvalStatus === "rejected") {
        return "bg-rose-500/15 text-rose-300";
    }
    if (tool.denyKind === "pending_approval") {
        return "bg-amber-500/15 text-amber-300";
    }
    if (tool.denyKind === "fail_closed") {
        return "bg-red-500/15 text-red-300";
    }
    if (tool.denyKind === "policy_denied") {
        return "bg-orange-500/15 text-orange-300";
    }
    if (tool.denyKind === "hook_denied") {
        return "bg-fuchsia-500/15 text-fuchsia-300";
    }
    if (tool.denyKind === "execution_error") {
        return "bg-red-500/15 text-red-300";
    }
    return "bg-emerald-500/15 text-emerald-300";
}

function renderToolDetail(tool: ToolTraceItem, t: TranslateFn): string {
    if (tool.approvalStatus === "requested") {
        return `${tool.text}\n${t("chat.tools.detail.waiting_approval", { defaultValue: "等待用户审批后继续。" })}`;
    }

    if (tool.text.startsWith("Cue triggered:")) {
        const cueName = tool.text.slice("Cue triggered:".length).trim();
        return t("chat.tools.result.cue_triggered", {
            cue: cueName,
            defaultValue: `Cue triggered: ${cueName}`,
        });
    }

    return tool.text;
}

function getToolItemKey(tool: ToolTraceItem, idx: number): string {
    return [tool.tool, tool.approvalRequestId ?? "none", tool.approvalStatus ?? "none", idx].join(":");
}

function isToolTextError(tool: ToolTraceItem): boolean {
    return tool.isError === true && tool.approvalStatus !== "approved";
}

function isToolActionDisabled(_isStreaming: boolean, tool: ToolTraceItem): boolean {
    return !canResolvePendingTool(tool);
}

function isPendingToolWaiting(tool: ToolTraceItem): boolean {
    return canResolvePendingTool(tool);
}

function isPendingApprovalTrace(tool: ToolTraceItem): boolean {
    return tool.denyKind === "pending_approval";
}

function isToolSuccessful(tool: ToolTraceItem): boolean {
    return tool.isError !== true || tool.approvalStatus === "approved";
}

function getToolBorderClassName(tool: ToolTraceItem): string {
    return isToolSuccessful(tool) ? "border-slate-800/70" : "border-red-900/60";
}

function getToolTextClassName(tool: ToolTraceItem): string {
    return isToolTextError(tool) ? "text-red-300" : "";
}

const toolStatusLabel: Record<
    NonNullable<ToolTraceItem["denyKind"]>,
    { key: string; defaultValue: string }
> = {
    pending_approval: { key: "chat.tools.status.pending_approval", defaultValue: "Pending approval" },
    fail_closed: { key: "chat.tools.status.fail_closed", defaultValue: "Fail-closed" },
    policy_denied: { key: "chat.tools.status.policy_denied", defaultValue: "Policy denied" },
    hook_denied: { key: "chat.tools.status.hook_denied", defaultValue: "Hook denied" },
    execution_error: { key: "chat.tools.status.execution_error", defaultValue: "Error" },
};

export const ChatMessage = memo(function ChatMessage({
    message: msg,
    isStreaming,
    isTranslationExpanded,
    onToggleTranslation,
    onEdit,
    onRegenerate,
    onContinueFrom,
    onApproveTool,
    onRejectTool,
}: ChatMessageProps) {
    const { t } = useTranslation();
    const [isEditing, setIsEditing] = useState(false);
    const [editingText, setEditingText] = useState("");
    const [toolsExpanded, setToolsExpanded] = useState(false);

    const handleStartEdit = () => {
        setIsEditing(true);
        setEditingText(msg.text);
    };

    const handleSaveEdit = () => {
        onEdit(editingText);
        setIsEditing(false);
    };

    const handleCancelEdit = () => {
        setIsEditing(false);
        setEditingText("");
    };

    return (
        <motion.div
            initial={{ opacity: 0, y: 10, scale: 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            transition={{ duration: 0.3 }}
            className={clsx(
                "group relative max-w-[85%] p-3 rounded-lg text-sm leading-relaxed font-body",
                msg.role === "user"
                    ? "ml-auto bg-[var(--color-accent)]/10 border border-[var(--color-accent)]/30 text-[var(--color-accent)] rounded-tr-none"
                    : msg.role === "tool"
                        ? "mx-auto max-w-[92%] bg-slate-950/60 border border-slate-800/70 text-slate-400 rounded-md text-xs px-3 py-2"
                        : "mr-auto bg-slate-900/50 border border-slate-700/50 text-slate-300 rounded-tl-none"
            )}
        >
            {msg.images && msg.images.length > 0 && (
                <div className="flex flex-wrap gap-1.5 mb-2">
                    {msg.images.map((url, imgIdx) => (
                        <img
                            key={imgIdx}
                            src={url}
                            alt="attached"
                            className="max-w-[180px] max-h-[120px] rounded-md object-cover border border-white/10"
                        />
                    ))}
                </div>
            )}

            {/* 编辑模式 */}
            {isEditing ? (
                <div className="space-y-2">
                    <textarea
                        value={editingText}
                        onChange={(e) => setEditingText(e.target.value)}
                        className="w-full bg-black/40 border border-[var(--color-border)] text-[var(--color-text-primary)] rounded-md px-3 py-2 text-sm focus:outline-none focus:border-[var(--color-accent)] resize-none"
                        rows={3}
                        autoFocus
                        onKeyDown={(e) => {
                            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                                handleSaveEdit();
                            } else if (e.key === "Escape") {
                                handleCancelEdit();
                            }
                        }}
                    />
                    <div className="flex items-center gap-2">
                        <button
                            onClick={handleSaveEdit}
                            className="p-1.5 rounded-md bg-[var(--color-accent)]/20 text-[var(--color-accent)] hover:bg-[var(--color-accent)]/30 transition-colors"
                            title={t("chat.actions.save")}
                        >
                            <Check size={14} />
                        </button>
                        <button
                            onClick={handleCancelEdit}
                            className="p-1.5 rounded-md bg-slate-800/50 text-slate-400 hover:text-slate-300 transition-colors"
                            title={t("chat.actions.cancel")}
                        >
                            <X size={14} />
                        </button>
                    </div>
                </div>
            ) : (
                <>
                    <div className="whitespace-pre-wrap break-words">
                        {msg.text}
                    </div>

                    {/* 错误消息的重试按钮 */}
                    {!isStreaming && msg.isError && (
                        <div className="absolute top-2 right-2 flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                            <button
                                onClick={onRegenerate}
                                className="p-1.5 rounded-md bg-slate-800/90 backdrop-blur-sm text-red-400 hover:text-red-300 hover:bg-slate-700/90 transition-colors shadow-lg"
                                title={t("chat.actions.retry")}
                            >
                                <RefreshCw size={12} strokeWidth={1.5} />
                            </button>
                        </div>
                    )}

                    {/* 悬停显示的操作按钮 */}
                    {!isStreaming && !msg.isError && msg.role !== "tool" && (
                        <div className="absolute top-2 right-2 flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                            {/* 从这里继续按钮 */}
                            <button
                                onClick={onContinueFrom}
                                className="p-1.5 rounded-md bg-slate-800/90 backdrop-blur-sm text-slate-400 hover:text-[var(--color-accent)] hover:bg-slate-700/90 transition-colors shadow-lg"
                                title={t("chat.actions.continue_from")}
                            >
                                <CornerDownLeft size={12} strokeWidth={1.5} />
                            </button>

                            {/* 编辑按钮 */}
                            <button
                                onClick={handleStartEdit}
                                className="p-1.5 rounded-md bg-slate-800/90 backdrop-blur-sm text-slate-400 hover:text-[var(--color-accent)] hover:bg-slate-700/90 transition-colors shadow-lg"
                                title={t("chat.actions.edit")}
                            >
                                <Edit2 size={12} strokeWidth={1.5} />
                            </button>

                            {/* 重新生成按钮（仅 kokoro 消息） */}
                            {msg.role === "kokoro" && (
                                <button
                                    onClick={onRegenerate}
                                    className="p-1.5 rounded-md bg-slate-800/90 backdrop-blur-sm text-slate-400 hover:text-[var(--color-accent)] hover:bg-slate-700/90 transition-colors shadow-lg"
                                    title={t("chat.actions.regenerate")}
                                >
                                    <RefreshCw size={12} strokeWidth={1.5} />
                                </button>
                            )}
                        </div>
                    )}
                </>
            )}

            {/* Translation toggle */}
            {!isEditing && msg.role === "kokoro" && msg.translation && (
                <div className="mt-2 -mb-1">
                    <button
                        onClick={onToggleTranslation}
                        className="flex items-center gap-1 text-[10px] text-[var(--color-text-muted)] hover:text-[var(--color-accent)] transition-colors"
                    >
                        <Languages size={11} strokeWidth={1.5} />
                        {isTranslationExpanded ? t("chat.translation.hide") : t("chat.translation.show")}
                    </button>
                    <AnimatePresence>
                        {isTranslationExpanded && (
                            <motion.div
                                initial={{ height: 0, opacity: 0 }}
                                animate={{ height: "auto", opacity: 1 }}
                                exit={{ height: 0, opacity: 0 }}
                                transition={{ duration: 0.2 }}
                                className="overflow-hidden"
                            >
                                <div className="mt-1.5 pt-1.5 border-t border-slate-700/40 text-xs text-[var(--color-text-muted)] leading-relaxed whitespace-pre-wrap break-words">
                                    {msg.translation}
                                </div>
                            </motion.div>
                        )}
                    </AnimatePresence>
                </div>
            )}

            {!isEditing && msg.role === "kokoro" && msg.tools && msg.tools.length > 0 && (
                <div className="mt-2">
                    <button
                        onClick={() => setToolsExpanded(prev => !prev)}
                        className="flex items-center gap-1 text-[10px] text-[var(--color-text-muted)] hover:text-[var(--color-accent)] transition-colors"
                    >
                        <Wrench size={11} strokeWidth={1.5} />
                        {t("chat.tools.header", { count: msg.tools.length, defaultValue: `Tools (${msg.tools.length})` })}
                        <ChevronDown
                            size={11}
                            strokeWidth={1.5}
                            className={clsx("transition-transform", toolsExpanded && "rotate-180")}
                        />
                    </button>
                    <AnimatePresence initial={false}>
                        {toolsExpanded && (
                            <motion.div
                                initial={{ height: 0, opacity: 0 }}
                                animate={{ height: "auto", opacity: 1 }}
                                exit={{ height: 0, opacity: 0 }}
                                transition={{ duration: 0.2 }}
                                className="overflow-hidden"
                            >
                                <div className="mt-1.5 rounded-md border border-slate-700/40 bg-slate-950/40 px-2 py-2 text-[11px] text-slate-400 space-y-1">
                                    {msg.tools.map((tool, idx) => (
                                        <div
                                            key={getToolItemKey(tool, idx)}
                                            className={clsx("rounded border bg-slate-950/40 px-2 py-1.5", getToolBorderClassName(tool))}
                                        >
                                            <div className="mb-1 flex items-center justify-between gap-2 text-[10px]">
                                                <div className="flex items-center gap-2">
                                                    <span className={clsx(
                                                        "rounded px-1.5 py-0.5 font-medium",
                                                        getToolStatusClassName(tool)
                                                    )}>
                                                        {getToolStatusLabel(tool, t)}
                                                    </span>
                                                    <span className="text-slate-500">{tool.tool}</span>
                                                </div>
                                                {isPendingToolWaiting(tool) && (
                                                    <div className="flex items-center gap-1">
                                                        <button
                                                            type="button"
                                                            onClick={() => onApproveTool(tool)}
                                                            disabled={isToolActionDisabled(isStreaming, tool)}
                                                            className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-emerald-300 transition-colors hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-50"
                                                        >
                                                            {t("chat.tools.actions.approve", { defaultValue: "Approve" })}
                                                        </button>
                                                        <button
                                                            type="button"
                                                            onClick={() => onRejectTool(tool)}
                                                            disabled={isToolActionDisabled(isStreaming, tool)}
                                                            className="rounded bg-rose-500/15 px-1.5 py-0.5 text-rose-300 transition-colors hover:bg-rose-500/25 disabled:cursor-not-allowed disabled:opacity-50"
                                                        >
                                                            {t("chat.tools.actions.reject", { defaultValue: "Reject" })}
                                                        </button>
                                                    </div>
                                                )}
                                            </div>
                                            <div className={clsx("whitespace-pre-wrap break-words", getToolTextClassName(tool))}>
                                                {renderToolDetail(tool, t)}
                                            </div>
                                            {isPendingApprovalTrace(tool) && tool.approvalRequestId && (
                                                <div className="mt-1 text-[10px] text-slate-500">
                                                    {t("chat.tools.request_label", { defaultValue: "Request" })}: {tool.approvalRequestId}
                                                </div>
                                            )}
                                        </div>
                                    ))}
                                </div>
                            </motion.div>
                        )}
                    </AnimatePresence>
                </div>
            )}
        </motion.div>
    );
});
