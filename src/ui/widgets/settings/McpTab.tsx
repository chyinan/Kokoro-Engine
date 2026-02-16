import { useState, useEffect, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import {
    Server, Plus, Trash2, RefreshCw, CheckCircle2, XCircle,
    Loader2, Wrench, AlertCircle, Copy
} from "lucide-react";
import { useTranslation } from "react-i18next";
import {
    listMcpServers, addMcpServer, removeMcpServer, refreshMcpTools, reconnectMcpServer
} from "../../../lib/kokoro-bridge";
import type { McpServerConfig, McpServerStatus } from "../../../lib/kokoro-bridge";

// ── Helpers ─────────────────────────────────────────────

/**
 * Parse standard `mcpServers` JSON format into our McpServerConfig[].
 *
 * Accepts either:
 * 1. Full `{ "mcpServers": { "name": { ... } } }` structure
 * 2. Just the inner `{ "name": { ... }, "name2": { ... } }` object
 * 3. A single server entry `"name": { ... }` (wrapped in braces automatically)
 */
function parseMcpJson(raw: string): McpServerConfig[] {
    let trimmed = raw.trim();

    // If user pasted something like `"name": { ... }` without outer braces, wrap it
    if (trimmed.startsWith('"') && !trimmed.startsWith('{')) {
        trimmed = `{${trimmed}}`;
    }

    // Remove trailing comma if present (common copy-paste issue)
    trimmed = trimmed.replace(/,\s*$/, '');
    // Re-wrap if we stripped a comma from a single entry
    if (trimmed.startsWith('"') && !trimmed.startsWith('{')) {
        trimmed = `{${trimmed}}`;
    }

    const parsed = JSON.parse(trimmed);

    // Case 1: { "mcpServers": { ... } }
    const servers = parsed.mcpServers || parsed;

    // Convert to our format
    const configs: McpServerConfig[] = [];
    for (const [name, entry] of Object.entries(servers)) {
        const e = entry as any;
        configs.push({
            name,
            command: e.command || "",
            args: e.args || [],
            env: e.env || {},
            enabled: e.disabled === true ? false : true,
        });
    }

    return configs;
}

// ── Example placeholder ─────────────────────────────────
const EXAMPLE_JSON = `"my-server": {
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"],
  "transportType": "stdio"
}`;

// ── Component ───────────────────────────────────────────

export default function McpTab() {
    const { t } = useTranslation();
    const [servers, setServers] = useState<McpServerStatus[]>([]);
    const [loading, setLoading] = useState(true);
    const [refreshing, setRefreshing] = useState(false);
    const [showAdd, setShowAdd] = useState(false);
    const [jsonInput, setJsonInput] = useState("");
    const [parseError, setParseError] = useState<string | null>(null);
    const [adding, setAdding] = useState(false);
    const [removingName, setRemovingName] = useState<string | null>(null);
    const [successMsg, setSuccessMsg] = useState<string | null>(null);

    const fetchServers = useCallback(async () => {
        try {
            const list = await listMcpServers();
            setServers(list);
        } catch (e) {
            console.error("[McpTab] Failed to list servers:", e);
        } finally {
            setLoading(false);
        }
    }, []);

    useEffect(() => {
        fetchServers();
    }, [fetchServers]);

    // Auto-poll while any server is in "connecting" state
    useEffect(() => {
        const hasConnecting = servers.some(s => s.status === "connecting");
        if (!hasConnecting) return;
        const interval = setInterval(fetchServers, 2000);
        return () => clearInterval(interval);
    }, [servers, fetchServers]);

    // ── Add server(s) from JSON ──────────────────────────
    const handleAdd = async () => {
        setParseError(null);
        setSuccessMsg(null);

        let configs: McpServerConfig[];
        try {
            configs = parseMcpJson(jsonInput);
            if (configs.length === 0) {
                setParseError(t("settings.mcp.add_modal.placeholder_error"));
                return;
            }
        } catch (e) {
            setParseError(t("settings.mcp.add_modal.invalid_json", { error: e instanceof Error ? e.message : String(e) }));
            return;
        }

        setAdding(true);
        try {
            for (const cfg of configs) {
                await addMcpServer(cfg);
            }
            setSuccessMsg(t("settings.mcp.add_modal.success", { count: configs.length, names: configs.map(c => c.name).join(", ") }));
            setJsonInput("");
            setShowAdd(false);
            await fetchServers();
        } catch (e) {
            setParseError(t("settings.mcp.add_modal.failed_add", { error: e instanceof Error ? e.message : String(e) }));
        } finally {
            setAdding(false);
        }
    };

    // ── Remove server ────────────────────────────────────
    const handleRemove = async (name: string) => {
        setRemovingName(name);
        try {
            await removeMcpServer(name);
            await fetchServers();
        } catch (e) {
            console.error("[McpTab] Remove failed:", e);
        } finally {
            setRemovingName(null);
        }
    };

    // ── Reconnect server ─────────────────────────────────
    const handleReconnect = async (name: string) => {
        try {
            await reconnectMcpServer(name);
            await fetchServers(); // will now show "connecting" state
        } catch (e) {
            console.error("[McpTab] Reconnect failed:", e);
        }
    };

    // ── Refresh tools ────────────────────────────────────
    const handleRefresh = async () => {
        setRefreshing(true);
        try {
            await refreshMcpTools();
            await fetchServers();
        } catch (e) {
            console.error("[McpTab] Refresh failed:", e);
        } finally {
            setRefreshing(false);
        }
    };

    // Dismiss success after 3s
    useEffect(() => {
        if (successMsg) {
            const t = setTimeout(() => setSuccessMsg(null), 3000);
            return () => clearTimeout(t);
        }
    }, [successMsg]);

    if (loading) {
        return (
            <div className="flex items-center justify-center py-12 text-[var(--color-text-muted)]">
                <Loader2 size={20} className="animate-spin mr-2" /> {t("settings.mcp.loading")}
            </div>
        );
    }

    return (
        <div className="space-y-5">
            {/* Header row */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <Server size={16} strokeWidth={1.5} className="text-[var(--color-accent)]" />
                    <div>
                        <div className="text-sm font-heading font-semibold text-[var(--color-text-primary)]">
                            {t("settings.mcp.title")}
                        </div>
                        <div className="text-xs text-[var(--color-text-muted)]">
                            {t("settings.mcp.desc")}
                        </div>
                    </div>
                </div>
                <div className="flex items-center gap-2">
                    <motion.button
                        whileTap={{ scale: 0.95 }}
                        onClick={handleRefresh}
                        disabled={refreshing}
                        className={clsx(
                            "p-2 rounded-lg transition-colors",
                            "text-[var(--color-text-muted)] hover:text-[var(--color-accent)]",
                            "border border-[var(--color-border)] hover:border-[var(--color-accent)]",
                            refreshing && "opacity-50 cursor-not-allowed"
                        )}
                        title={t("settings.mcp.refresh_tooltip")}
                    >
                        <RefreshCw size={14} strokeWidth={1.5} className={refreshing ? "animate-spin" : ""} />
                    </motion.button>
                    <motion.button
                        whileTap={{ scale: 0.95 }}
                        onClick={() => setShowAdd(!showAdd)}
                        className={clsx(
                            "flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs font-heading font-semibold tracking-wider uppercase transition-colors",
                            showAdd
                                ? "bg-[var(--color-accent)] text-black"
                                : "border border-[var(--color-border)] text-[var(--color-text-secondary)] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]"
                        )}
                    >
                        <Plus size={14} strokeWidth={1.5} />
                        {t("settings.mcp.add_button")}
                    </motion.button>
                </div>
            </div>

            {/* Success message */}
            <AnimatePresence>
                {successMsg && (
                    <motion.div
                        initial={{ opacity: 0, y: -5 }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={{ opacity: 0 }}
                        className="flex items-center gap-2 px-3 py-2 rounded-lg bg-emerald-900/30 border border-emerald-500/30 text-emerald-300 text-xs"
                    >
                        <CheckCircle2 size={14} /> {successMsg}
                    </motion.div>
                )}
            </AnimatePresence>

            {/* Server list */}
            {servers.length === 0 ? (
                <div className="rounded-lg bg-[var(--color-bg-surface)] border border-[var(--color-border)] p-6 text-center">
                    <Server size={24} strokeWidth={1} className="mx-auto mb-2 text-[var(--color-text-muted)]" />
                    <p className="text-sm text-[var(--color-text-muted)]">
                        {t("settings.mcp.empty.title")}
                    </p>
                    <p
                        className="text-xs text-[var(--color-text-muted)] mt-1"
                        dangerouslySetInnerHTML={{ __html: t("settings.mcp.empty.desc") }}
                    />
                </div>
            ) : (
                <div className="space-y-2">
                    {servers.map((srv) => (
                        <motion.div
                            key={srv.name}
                            layout
                            initial={{ opacity: 0, y: 5 }}
                            animate={{ opacity: 1, y: 0 }}
                            className={clsx(
                                "flex items-center justify-between px-4 py-3 rounded-lg",
                                "bg-[var(--color-bg-surface)] border",
                                srv.status === "connected"
                                    ? "border-emerald-500/30"
                                    : srv.status === "connecting"
                                        ? "border-amber-500/30"
                                        : "border-red-500/30"
                            )}
                        >
                            <div className="flex items-center gap-3 min-w-0">
                                {/* Status indicator */}
                                {srv.status === "connected" ? (
                                    <CheckCircle2 size={16} className="text-emerald-400 flex-shrink-0" />
                                ) : srv.status === "connecting" ? (
                                    <Loader2 size={16} className="text-amber-400 animate-spin flex-shrink-0" />
                                ) : (
                                    <XCircle size={16} className="text-red-400 flex-shrink-0" />
                                )}
                                <div className="min-w-0">
                                    <div className="text-sm font-heading font-semibold text-[var(--color-text-primary)] truncate">
                                        {srv.name}
                                    </div>
                                    <div className="flex items-center gap-2 text-xs text-[var(--color-text-muted)]">
                                        {srv.status === "connected" ? (
                                            <>
                                                <span className="flex items-center gap-1">
                                                    <Wrench size={10} /> {srv.tool_count} {t("settings.mcp.status.tools", { count: srv.tool_count })}
                                                </span>
                                                {srv.server_version && (
                                                    <span className="opacity-60">v{srv.server_version}</span>
                                                )}
                                            </>
                                        ) : srv.status === "connecting" ? (
                                            <span className="text-amber-400">{t("settings.mcp.status.connecting")}</span>
                                        ) : (
                                            <span className="text-red-400" title={srv.error || undefined}>
                                                {srv.error ? t("settings.mcp.status.error", { error: srv.error }) : t("settings.mcp.status.disconnected")}
                                            </span>
                                        )}
                                    </div>
                                </div>
                            </div>
                            <div className="flex items-center gap-1 flex-shrink-0">
                                {/* Retry button — visible when disconnected */}
                                {srv.status === "disconnected" && (
                                    <motion.button
                                        whileTap={{ scale: 0.9 }}
                                        onClick={() => handleReconnect(srv.name)}
                                        className="p-2 rounded-md transition-colors text-[var(--color-text-muted)] hover:text-amber-400"
                                        title={t("settings.mcp.status.retry")}
                                    >
                                        <RefreshCw size={14} strokeWidth={1.5} />
                                    </motion.button>
                                )}
                                {/* Delete button */}
                                <motion.button
                                    whileTap={{ scale: 0.9 }}
                                    onClick={() => handleRemove(srv.name)}
                                    disabled={removingName === srv.name}
                                    className={clsx(
                                        "p-2 rounded-md transition-colors flex-shrink-0",
                                        "text-[var(--color-text-muted)] hover:text-red-400",
                                        removingName === srv.name && "opacity-50 cursor-not-allowed"
                                    )}
                                    title={t("settings.mcp.status.remove")}
                                >
                                    {removingName === srv.name ? (
                                        <Loader2 size={14} className="animate-spin" />
                                    ) : (
                                        <Trash2 size={14} strokeWidth={1.5} />
                                    )}
                                </motion.button>
                            </div>
                        </motion.div>
                    ))}
                </div>
            )}

            {/* Add server panel */}
            <AnimatePresence>
                {showAdd && (
                    <motion.div
                        initial={{ opacity: 0, height: 0 }}
                        animate={{ opacity: 1, height: "auto" }}
                        exit={{ opacity: 0, height: 0 }}
                        className="overflow-hidden"
                    >
                        <div className="space-y-3 rounded-lg bg-[var(--color-bg-surface)] border border-[var(--color-border)] p-4">
                            <div className="flex items-center gap-2 text-sm text-[var(--color-text-primary)] font-heading font-semibold">
                                <Plus size={14} /> {t("settings.mcp.add_modal.title")}
                            </div>

                            <p
                                className="text-xs text-[var(--color-text-muted)] leading-relaxed"
                                dangerouslySetInnerHTML={{ __html: t("settings.mcp.add_modal.desc") }}
                            />

                            <textarea
                                value={jsonInput}
                                onChange={(e) => {
                                    setJsonInput(e.target.value);
                                    setParseError(null);
                                }}
                                placeholder={EXAMPLE_JSON}
                                rows={8}
                                spellCheck={false}
                                className={clsx(
                                    "w-full px-3 py-2 rounded-lg text-xs font-mono leading-relaxed",
                                    "bg-black/40 border",
                                    parseError
                                        ? "border-red-500/50"
                                        : "border-[var(--color-border)]",
                                    "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]/40",
                                    "focus:outline-none focus:border-[var(--color-accent)] transition-colors",
                                    "resize-y"
                                )}
                            />

                            {/* Parse error */}
                            <AnimatePresence>
                                {parseError && (
                                    <motion.div
                                        initial={{ opacity: 0, y: -5 }}
                                        animate={{ opacity: 1, y: 0 }}
                                        exit={{ opacity: 0 }}
                                        className="flex items-start gap-2 text-xs text-red-400"
                                    >
                                        <AlertCircle size={14} className="flex-shrink-0 mt-0.5" />
                                        <span>{parseError}</span>
                                    </motion.div>
                                )}
                            </AnimatePresence>

                            <div className="flex items-center justify-between">
                                <button
                                    onClick={() => setJsonInput(EXAMPLE_JSON)}
                                    className="flex items-center gap-1.5 text-xs text-[var(--color-text-muted)] hover:text-[var(--color-accent)] transition-colors"
                                >
                                    <Copy size={12} /> {t("settings.mcp.add_modal.insert_example")}
                                </button>
                                <motion.button
                                    whileTap={{ scale: 0.97 }}
                                    onClick={handleAdd}
                                    disabled={adding || !jsonInput.trim()}
                                    className={clsx(
                                        "flex items-center gap-2 px-4 py-2 rounded-lg text-xs font-heading font-semibold tracking-wider uppercase",
                                        "bg-[var(--color-accent)] text-black",
                                        "hover:bg-white transition-colors",
                                        (adding || !jsonInput.trim()) && "opacity-50 cursor-not-allowed"
                                    )}
                                >
                                    {adding ? (
                                        <Loader2 size={14} className="animate-spin" />
                                    ) : (
                                        <Plus size={14} strokeWidth={2} />
                                    )}
                                    {adding ? t("settings.mcp.add_modal.adding") : t("settings.mcp.add_modal.add_server")}
                                </motion.button>
                            </div>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>

            {/* Info note */}
            <div className="rounded-lg bg-[var(--color-bg-surface)] border border-[var(--color-border)] p-3">
                <p
                    className="text-xs text-[var(--color-text-muted)] leading-relaxed"
                    dangerouslySetInnerHTML={{ __html: t("settings.mcp.info_note") }}
                />
            </div>
        </div>
    );
}
