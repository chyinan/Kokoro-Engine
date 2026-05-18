// pattern: Mixed (unavoidable)
// Reason: React 组件同时承载展示逻辑、事件订阅、状态更新与 IPC 调用，当前阶段只做最小前端接线。
import { useState, useEffect, useCallback, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { useTranslation } from "react-i18next";
import { Trash2, Pencil, Check, X, Search, Brain, ChevronDown, List, Calendar, Share2, UserCircle, Moon, Play, Clock } from "lucide-react";
import { inputClasses } from "../styles/settings-primitives";
import { Select } from "@/components/ui/select";
import {
    listMemories,
    updateMemory,
    deleteMemory,
    listCharacters,
    getMemoryEnabled,
    setMemoryEnabled,
    getDreamingSummary,
    listDreamJobs,
    listDreamProposals,
    runDreamNow,
    approveDreamProposal,
    rejectDreamProposal,
    getMemoryUpgradeConfig,
    setMemoryUpgradeConfig,
} from "../../lib/kokoro-bridge";
import type {
    MemoryRecord,
    CharacterRecord,
    MemoryUpgradeConfig,
    MemoryDreamingSummary,
    MemoryDreamJobRecord,
    MemoryDreamProposalRecord,
} from "../../lib/kokoro-bridge";
import { listen } from "@tauri-apps/api/event";
import { MemoryTimeline } from "./memory/MemoryTimeline";
import { MemoryGraph } from "./memory/MemoryGraph";
import {
    restoreStructuredMemoryPrefix,
    splitStructuredMemoryContent,
    stripStructuredMemoryPrefix,
} from "./memory/memory-display-content";

interface MemoryPanelProps {
    characterId: string;
}

type ViewMode = "list" | "timeline" | "graph" | "dream";

const DREAM_PROPOSAL_TITLE_KEYS: Record<string, string> = {
    "Review possible memory conflict": "settings.memory.dream.proposal_titles.conflict_review",
    "Review similar memories": "settings.memory.dream.proposal_titles.semantic_review",
    "Merged exact duplicate memories": "settings.memory.dream.proposal_titles.canonical_duplicate",
    "Merged duplicate slot memories": "settings.memory.dream.proposal_titles.entity_slot_merge",
    "Merged LLM-confirmed memories": "settings.memory.dream.proposal_titles.llm_semantic_auto_merge",
    "Review LLM-detected memory conflict": "settings.memory.dream.proposal_titles.llm_conflict_review",
    "Review LLM memory merge suggestion": "settings.memory.dream.proposal_titles.llm_semantic_review",
    "Merged highly similar memories": "settings.memory.dream.proposal_titles.semantic_auto_merge",
    "Review LLM-discovered memory relation": "settings.memory.dream.proposal_titles.llm_discovery_review",
    "Review LLM-discovered memory conflict": "settings.memory.dream.proposal_titles.llm_discovery_conflict_review",
    "Review LLM-discovered memory update": "settings.memory.dream.proposal_titles.llm_discovery_update_review",
};

const DREAM_PROPOSAL_RATIONALE_KEYS: Record<string, string> = {
    "A new memory appears to contradict an existing active memory.": "settings.memory.dream.proposal_rationales.conflict_review",
    "These memories are semantically similar but below the automatic merge threshold.": "settings.memory.dream.proposal_rationales.semantic_review",
    "Dream Light found duplicate canonical hashes.": "settings.memory.dream.proposal_rationales.canonical_duplicate",
    "Dream REM found multiple active memories for the same structured slot.": "settings.memory.dream.proposal_rationales.entity_slot_merge",
    "LLM confirmed these memories are safely mergeable.": "settings.memory.dream.proposal_rationales.llm_semantic_auto_merge",
    "LLM detected a possible contradiction.": "settings.memory.dream.proposal_rationales.llm_conflict_review",
    "LLM suggested reviewing these similar memories.": "settings.memory.dream.proposal_rationales.llm_semantic_review",
    "Dream Deep found a high-confidence semantic duplicate.": "settings.memory.dream.proposal_rationales.semantic_auto_merge",
    "LLM Dream Discovery found a non-obvious relationship between these memories.": "settings.memory.dream.proposal_rationales.llm_discovery_review",
    "LLM Dream Discovery found a non-obvious possible conflict between these memories.": "settings.memory.dream.proposal_rationales.llm_discovery_conflict_review",
    "LLM Dream Discovery found a non-obvious possible update between these memories.": "settings.memory.dream.proposal_rationales.llm_discovery_update_review",
};

const DREAM_PROPOSAL_IMPACT_KEYS: Record<string, string> = {
    "Manual review required before invalidating or replacing the older memory.": "settings.memory.dream.proposal_impacts.conflict_review",
    "Manual review required before superseding either memory.": "settings.memory.dream.proposal_impacts.semantic_review",
    "Auto-merged duplicate active memories; source rows were marked superseded.": "settings.memory.dream.proposal_impacts.auto_merged",
    "Manual review required before changing either active memory.": "settings.memory.dream.proposal_impacts.llm_conflict_review",
    "Manual review required because confidence was below the auto-apply threshold.": "settings.memory.dream.proposal_impacts.llm_semantic_review",
    "Manual review required because this relation was discovered by LLM rather than deterministic similarity.": "settings.memory.dream.proposal_impacts.llm_discovery_review",
};

const DREAM_PROPOSAL_TYPE_TITLE_KEYS: Record<string, string> = {
    llm_discovery_review: "settings.memory.dream.proposal_titles.llm_discovery_review",
    llm_discovery_conflict_review: "settings.memory.dream.proposal_titles.llm_discovery_conflict_review",
    llm_discovery_update_review: "settings.memory.dream.proposal_titles.llm_discovery_update_review",
};

const DREAM_PROPOSAL_TYPE_RATIONALE_KEYS: Record<string, string> = {
    llm_discovery_review: "settings.memory.dream.proposal_rationales.llm_discovery_review",
    llm_discovery_conflict_review: "settings.memory.dream.proposal_rationales.llm_discovery_conflict_review",
    llm_discovery_update_review: "settings.memory.dream.proposal_rationales.llm_discovery_update_review",
};

const DREAM_PROPOSAL_TYPE_IMPACT_KEYS: Record<string, string> = {
    llm_discovery_review: "settings.memory.dream.proposal_impacts.llm_discovery_review",
    llm_discovery_conflict_review: "settings.memory.dream.proposal_impacts.llm_discovery_review",
    llm_discovery_update_review: "settings.memory.dream.proposal_impacts.llm_discovery_review",
};

const DREAM_HOUR_OPTIONS = Array.from({ length: 24 }, (_, hour) => ({
    value: String(hour),
    label: `${hour.toString().padStart(2, "0")}:00`,
}));

export default function MemoryPanel({ characterId }: MemoryPanelProps) {
    const { t } = useTranslation();
    const [view, setView] = useState<ViewMode>("list");
    const [memories, setMemories] = useState<MemoryRecord[]>([]);
    const [total, setTotal] = useState(0);
    const [loading, setLoading] = useState(false);
    const [searchQuery, setSearchQuery] = useState("");
    const [editingId, setEditingId] = useState<number | null>(null);
    const [editContent, setEditContent] = useState("");
    const [editContentPrefix, setEditContentPrefix] = useState<string | null>(null);
    const [editImportance, setEditImportance] = useState(0.5);
    const [deletingId, setDeletingId] = useState<number | null>(null);
    const [page, setPage] = useState(0);
    const [memoryEnabled, setMemoryEnabledState] = useState(true);
    const [togglingMemory, setTogglingMemory] = useState(false);
    const [dreamingSummary, setDreamingSummary] = useState<MemoryDreamingSummary | null>(null);
    const [dreamJobs, setDreamJobs] = useState<MemoryDreamJobRecord[]>([]);
    const [dreamProposals, setDreamProposals] = useState<MemoryDreamProposalRecord[]>([]);
    const [dreamLoading, setDreamLoading] = useState(false);
    const [dreamRunning, setDreamRunning] = useState(false);
    const [dreamActionId, setDreamActionId] = useState<number | null>(null);
    const [dreamError, setDreamError] = useState<string | null>(null);
    const [dreamConfig, setDreamConfig] = useState<MemoryUpgradeConfig | null>(null);
    const [dreamConfigLoading, setDreamConfigLoading] = useState(false);
    const [dreamConfigSaving, setDreamConfigSaving] = useState(false);
    const [dreamConfigSaved, setDreamConfigSaved] = useState(false);
    const pageSize = 50; // Load more for graph/timeline

    // ── Character selector state ──
    const [characters, setCharacters] = useState<CharacterRecord[]>([]);
    const [selectedCharId, setSelectedCharId] = useState<string>(characterId);

    // Load character list for the dropdown
    useEffect(() => {
        listCharacters().then((all) => {
            setCharacters(all);
            if (!all.find((c) => c.id === characterId) && all.length > 0) {
                setSelectedCharId(all[0].id);
            }
        }).catch((e) => console.error("[MemoryPanel] Failed to load characters:", e));
    }, [characterId]);

    useEffect(() => {
        getMemoryEnabled()
            .then(setMemoryEnabledState)
            .catch((e) => console.error("[MemoryPanel] Failed to load memory toggle:", e));
    }, []);

    const fetchDreaming = useCallback(async (showLoading = true) => {
        if (!selectedCharId) return;
        if (showLoading) {
            setDreamLoading(true);
        }
        setDreamError(null);
        try {
            const [summary, jobs, proposals] = await Promise.all([
                getDreamingSummary(selectedCharId),
                listDreamJobs(selectedCharId, 5),
                listDreamProposals(selectedCharId, "pending", 50),
            ]);
            setDreamingSummary(summary);
            setDreamJobs(jobs);
            setDreamProposals(proposals);
        } catch (e) {
            setDreamError(typeof e === "string" ? e : ((e as any)?.message ?? JSON.stringify(e)));
        } finally {
            if (showLoading) {
                setDreamLoading(false);
            }
        }
    }, [selectedCharId]);

    useEffect(() => {
        fetchDreaming();
    }, [fetchDreaming]);

    useEffect(() => {
        if (view !== "dream") return;
        void fetchDreaming(false);
    }, [view, fetchDreaming]);

    useEffect(() => {
        if (view !== "dream" || !selectedCharId) return;
        const latestStatus = dreamingSummary?.latest_job?.status;
        const refreshMs = dreamRunning || latestStatus === "running" ? 2500 : 10000;
        const intervalId = window.setInterval(() => {
            void fetchDreaming(false);
        }, refreshMs);
        return () => window.clearInterval(intervalId);
    }, [view, selectedCharId, dreamRunning, dreamingSummary?.latest_job?.status, fetchDreaming]);

    useEffect(() => {
        setDreamConfigLoading(true);
        getMemoryUpgradeConfig()
            .then(setDreamConfig)
            .catch((e) => {
                setDreamError(typeof e === "string" ? e : ((e as any)?.message ?? JSON.stringify(e)));
            })
            .finally(() => setDreamConfigLoading(false));
    }, []);

    // Reset page when switching characters
    useEffect(() => {
        setPage(0);
        setSearchQuery("");
    }, [selectedCharId]);

    const fetchMemories = useCallback(async () => {
        console.log("[MemoryPanel] fetchMemories called with selectedCharId:", selectedCharId);
        if (!selectedCharId) return;
        setLoading(true);
        try {
            const res = await listMemories(selectedCharId, pageSize, page * pageSize);
            setMemories(res.memories);
            setTotal(res.total);
        } catch (e) {
            console.error("[MemoryPanel] Failed to load memories:", e);
        } finally {
            setLoading(false);
        }
    }, [selectedCharId, page]);

    useEffect(() => {
        fetchMemories();
    }, [fetchMemories]);

    // Auto-refresh when backend writes a new memory (e.g. via Telegram tool call)
    const fetchMemoriesRef = useRef(fetchMemories);
    useEffect(() => { fetchMemoriesRef.current = fetchMemories; }, [fetchMemories]);
    useEffect(() => {
        const unlisten = listen<string>("memory:updated", (event) => {
            // Only refresh if the updated character matches the currently viewed one
            if (!event.payload || event.payload === selectedCharId) {
                fetchMemoriesRef.current();
            }
        });
        return () => { unlisten.then(fn => fn()); };
    }, [selectedCharId]);

    // Filter client-side by search query (simple text match)
    const filtered = searchQuery.trim()
        ? memories.filter((m) =>
            stripStructuredMemoryPrefix(m.content).toLowerCase().includes(searchQuery.toLowerCase())
        )
        : memories;

    const startEdit = (mem: MemoryRecord) => {
        const parsedContent = splitStructuredMemoryContent(mem.content);
        setEditingId(mem.id);
        setEditContent(parsedContent.text);
        setEditContentPrefix(parsedContent.prefix);
        setEditImportance(mem.importance);
    };

    const saveEdit = async () => {
        if (editingId === null) return;
        try {
            await updateMemory(
                editingId,
                restoreStructuredMemoryPrefix(editContentPrefix, editContent),
                editImportance,
            );
            setEditingId(null);
            setEditContentPrefix(null);
            fetchMemories();
        } catch (e) {
            console.error("[MemoryPanel] Failed to update memory:", e);
        }
    };

    const confirmDelete = async (id: number) => {
        try {
            await deleteMemory(id);
            setDeletingId(null);
            fetchMemories();
        } catch (e) {
            console.error("[MemoryPanel] Failed to delete memory:", e);
        }
    };

    const hasMore = (page + 1) * pageSize < total;

    const handleToggleMemory = async () => {
        const next = !memoryEnabled;
        setTogglingMemory(true);
        try {
            await setMemoryEnabled(next);
            setMemoryEnabledState(next);
        } catch (e) {
            console.error("[MemoryPanel] Failed to toggle memory system:", e);
        } finally {
            setTogglingMemory(false);
        }
    };

    const handleRunDream = async () => {
        if (!selectedCharId) return;
        setDreamRunning(true);
        setDreamError(null);
        try {
            await runDreamNow(selectedCharId);
            await Promise.all([fetchDreaming(), fetchMemories()]);
        } catch (e) {
            setDreamError(typeof e === "string" ? e : ((e as any)?.message ?? JSON.stringify(e)));
        } finally {
            setDreamRunning(false);
        }
    };

    const handleDreamProposal = async (id: number, action: "approve" | "reject") => {
        setDreamActionId(id);
        setDreamError(null);
        try {
            if (action === "approve") {
                await approveDreamProposal(id);
            } else {
                await rejectDreamProposal(id);
            }
            await Promise.all([fetchDreaming(), fetchMemories()]);
        } catch (e) {
            setDreamError(typeof e === "string" ? e : ((e as any)?.message ?? JSON.stringify(e)));
        } finally {
            setDreamActionId(null);
        }
    };

    const handleDreamHourChange = async (value: string) => {
        if (!dreamConfig) return;
        const hour = Number.parseInt(value, 10);
        if (!Number.isInteger(hour) || hour < 0 || hour > 23) return;
        const nextConfig: MemoryUpgradeConfig = {
            ...dreamConfig,
            dream_daily_hour: hour,
        };
        setDreamConfig(nextConfig);
        setDreamConfigSaving(true);
        setDreamConfigSaved(false);
        setDreamError(null);
        try {
            await setMemoryUpgradeConfig(nextConfig);
            setDreamConfigSaved(true);
            window.setTimeout(() => setDreamConfigSaved(false), 1600);
        } catch (e) {
            setDreamError(typeof e === "string" ? e : ((e as any)?.message ?? JSON.stringify(e)));
        } finally {
            setDreamConfigSaving(false);
        }
    };

    // Helpers
    const getTimeAgo = (ts: number) => {
        const now = Date.now() / 1000;
        const diff = now - ts;
        if (diff < 60) return t("settings.memory.time.just_now");
        if (diff < 3600) return t("settings.memory.time.minutes_ago", { count: Math.floor(diff / 60) });
        if (diff < 86400) return t("settings.memory.time.hours_ago", { count: Math.floor(diff / 3600) });
        if (diff < 604800) return t("settings.memory.time.days_ago", { count: Math.floor(diff / 86400) });
        return new Date(ts * 1000).toLocaleDateString();
    };

    const getImportanceLabel = (v: number) => {
        if (v >= 0.7) return t("settings.memory.importance.high");
        if (v >= 0.4) return t("settings.memory.importance.med");
        return t("settings.memory.importance.low");
    };

    const getImportanceColor = (v: number) => {
        if (v >= 0.7) return "text-red-400 bg-red-500/15 border-red-500/30";
        if (v >= 0.4) return "text-amber-400 bg-amber-500/15 border-amber-500/30";
        return "text-[var(--color-accent)] bg-[var(--color-accent)]/15 border-[var(--color-accent)]/30";
    };

    const formatConfidence = (value: number) => `${Math.round(value * 100)}%`;

    const parseProposalIdList = (raw: string) => {
        try {
            const parsed = JSON.parse(raw);
            return Array.isArray(parsed)
                ? parsed.filter((value): value is number => typeof value === "number")
                : [];
        } catch {
            return [];
        }
    };

    const translateDreamCode = (group: string, code: string) =>
        t(`settings.memory.dream.${group}.${code}`, { defaultValue: code });

    const formatProposalTitle = (proposal: MemoryDreamProposalRecord) => {
        const key = DREAM_PROPOSAL_TYPE_TITLE_KEYS[proposal.proposal_type]
            ?? DREAM_PROPOSAL_TITLE_KEYS[proposal.title];
        return key ? t(key) : proposal.title;
    };

    const formatProposalRationale = (proposal: MemoryDreamProposalRecord) => {
        const key = DREAM_PROPOSAL_TYPE_RATIONALE_KEYS[proposal.proposal_type]
            ?? DREAM_PROPOSAL_RATIONALE_KEYS[proposal.rationale];
        return key ? t(key) : proposal.rationale;
    };

    const formatProposalImpact = (proposal: MemoryDreamProposalRecord) => {
        const key = DREAM_PROPOSAL_TYPE_IMPACT_KEYS[proposal.proposal_type]
            ?? DREAM_PROPOSAL_IMPACT_KEYS[proposal.impact];
        return key ? t(key) : proposal.impact;
    };

    return (
        <div className="space-y-4 flex flex-col">
            {/* Header & Character Selector */}
            <div className="flex flex-col gap-4 shrink-0">
                <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                        <Brain
                            size={16}
                            className="text-[var(--color-accent)]"
                        />
                        <span className="text-xs font-heading font-bold uppercase tracking-wider text-[var(--color-text-muted)]">
                            {t("settings.memory.title")}
                        </span>
                    </div>
                    <span className="text-[10px] font-mono text-[var(--color-text-muted)]">
                        {t("settings.memory.count", { count: total })}
                    </span>
                </div>
                <div className="rounded-lg border border-[var(--color-border)] bg-black/20 p-3">
                    <div className="flex items-center justify-between gap-4">
                        <div className="min-w-0 flex-1 space-y-1">
                            <div className="text-sm font-heading font-semibold text-[var(--color-text-primary)]">
                                {t("settings.memory.toggle.label")}
                            </div>
                            <p className="text-xs text-[var(--color-text-muted)] leading-relaxed">
                                {t("settings.memory.toggle.desc")}
                            </p>
                        </div>
                        <button
                            onClick={handleToggleMemory}
                            disabled={togglingMemory}
                            className={clsx(
                                "relative h-6 w-11 shrink-0 rounded-full transition-colors disabled:opacity-60",
                                memoryEnabled ? "bg-[var(--color-accent)]" : "bg-[var(--color-border)]"
                            )}
                            role="switch"
                            aria-checked={memoryEnabled}
                            aria-label={t("settings.memory.toggle.label")}
                        >
                            <span
                                className={clsx(
                                    "absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white transition-transform",
                                    memoryEnabled && "translate-x-5"
                                )}
                            />
                        </button>
                    </div>
                    {!memoryEnabled && (
                        <div className="mt-3 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-200">
                            {t("settings.memory.toggle.disabled_hint")}
                        </div>
                    )}
                </div>

                {/* Character Selector */}
                {characters.length > 0 && (
                    <div className="relative">
                        <UserCircle
                            size={14}
                            className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--color-text-muted)] z-10 pointer-events-none"
                        />
                        <Select
                            value={selectedCharId}
                            onChange={setSelectedCharId}
                            options={characters
                                .filter(char => char.id === characterId || !characters.some(c => c.name === char.name && c.id === characterId))
                                .map(char => ({
                                    value: char.id,
                                    label: `${char.name}${char.id === characterId ? ` ${t("settings.memory.active_char")}` : ""}`,
                                }))}
                            className="[&>button]:pl-9"
                        />
                    </div>
                )}

                <div className="flex bg-black/20 rounded-lg p-1 gap-1 border border-white/5">
                    {[
                        { id: "list", label: t("settings.memory.tabs.list"), icon: List },
                        { id: "timeline", label: t("settings.memory.tabs.timeline"), icon: Calendar },
                        { id: "graph", label: t("settings.memory.tabs.graph"), icon: Share2 },
                        { id: "dream", label: t("settings.memory.tabs.dream"), icon: Moon },
                    ].map(tab => (
                        <button
                            key={tab.id}
                            onClick={() => setView(tab.id as ViewMode)}
                            className={clsx(
                                "flex-1 flex items-center justify-center gap-2 py-1.5 rounded text-xs transition-colors",
                                view === tab.id
                                    ? "bg-[var(--color-accent)]/20 text-[var(--color-accent)] font-medium"
                                    : "text-[var(--color-text-muted)] hover:bg-white/5 hover:text-slate-200"
                            )}
                        >
                            <tab.icon size={12} />
                            {tab.label}
                        </button>
                    ))}
                </div>

                {/* Search */}
                {view !== "dream" && (
                    <div className="relative">
                        <Search
                            size={14}
                            className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--color-text-muted)]"
                        />
                        <input
                            type="text"
                            value={searchQuery}
                            onChange={(e) => setSearchQuery(e.target.value)}
                            placeholder={t("settings.memory.search.placeholder")}
                            className={clsx(inputClasses, "pl-9 py-2")}
                        />
                    </div>
                )}
            </div>

            {/* Content Area */}
            <div className="flex-1 overflow-y-auto min-h-0 relative scrollable pr-1">
                {view === "dream" ? (
                    <div className="space-y-3 pb-4">
                        <div className="rounded-lg border border-[var(--color-border)] bg-black/20 p-3" aria-busy={dreamLoading}>
                            <div className="flex flex-wrap items-center justify-between gap-3">
                                <div className="flex items-center gap-2">
                                    <Moon
                                        size={15}
                                        className={clsx("text-[var(--color-accent)]", dreamLoading && "animate-pulse")}
                                    />
                                    <div>
                                        <div className="text-sm font-heading font-semibold text-[var(--color-text-primary)]">
                                            {t("settings.memory.dream.title")}
                                        </div>
                                        <div className="text-[10px] text-[var(--color-text-muted)]">
                                            {t("settings.memory.dream.summary", {
                                                pending: dreamingSummary?.pending_proposal_count ?? 0,
                                                auto: dreamingSummary?.auto_applied_proposal_count ?? 0,
                                            })}
                                        </div>
                                    </div>
                                </div>
                                <div className="flex items-center gap-2">
                                    <button
                                        onClick={handleRunDream}
                                        disabled={dreamRunning || !memoryEnabled}
                                        className="flex items-center gap-2 rounded-md border border-[var(--color-accent)]/40 bg-[var(--color-accent)]/15 px-3 py-2 text-xs text-[var(--color-accent)] hover:bg-[var(--color-accent)]/25 disabled:opacity-50"
                                    >
                                        <Play size={13} />
                                        {dreamRunning
                                            ? t("settings.memory.dream.running")
                                            : t("settings.memory.dream.run_now")}
                                    </button>
                                </div>
                            </div>
                            <div className="mt-3 rounded-md border border-[var(--color-border)] bg-black/20 px-3 py-2">
                                <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                                    <div className="flex items-start gap-2">
                                        <Clock size={14} className="mt-0.5 text-[var(--color-accent)]" />
                                        <div>
                                            <div className="text-xs font-medium text-[var(--color-text-primary)]">
                                                {t("settings.memory.dream.schedule.title")}
                                            </div>
                                            <div className="mt-0.5 text-[10px] text-[var(--color-text-muted)]">
                                                {t("settings.memory.dream.schedule.hint", {
                                                    hour: (dreamConfig?.dream_daily_hour ?? 3).toString().padStart(2, "0"),
                                                })}
                                            </div>
                                        </div>
                                    </div>
                                    <div className="flex min-w-[160px] items-center gap-2">
                                        <Select
                                            value={String(dreamConfig?.dream_daily_hour ?? 3)}
                                            onChange={handleDreamHourChange}
                                            options={DREAM_HOUR_OPTIONS}
                                            disabled={dreamConfigLoading || dreamConfigSaving}
                                            className="min-w-[120px]"
                                        />
                                        <span className="min-w-[42px] text-[10px] text-[var(--color-text-muted)]">
                                            {dreamConfigSaving
                                                ? t("settings.memory.dream.schedule.saving")
                                                : dreamConfigSaved
                                                    ? t("settings.memory.dream.schedule.saved")
                                                    : ""}
                                        </span>
                                    </div>
                                </div>
                            </div>
                            {dreamingSummary?.latest_job && (
                                <div className="mt-3 grid gap-2 text-xs text-[var(--color-text-muted)] md:grid-cols-3">
                                    <div className="rounded-md border border-[var(--color-border)] bg-black/20 px-3 py-2">
                                        {t("settings.memory.dream.latest.last", {
                                            status: translateDreamCode("statuses", dreamingSummary.latest_job.status),
                                        })}
                                    </div>
                                    <div className="rounded-md border border-[var(--color-border)] bg-black/20 px-3 py-2">
                                        {t("settings.memory.dream.latest.auto_applied", {
                                            count: dreamingSummary.latest_job.auto_applied_count,
                                        })}
                                    </div>
                                    <div className="rounded-md border border-[var(--color-border)] bg-black/20 px-3 py-2">
                                        {t("settings.memory.dream.latest.proposals", {
                                            count: dreamingSummary.latest_job.proposal_count,
                                        })}
                                    </div>
                                </div>
                            )}
                            {dreamError && (
                                <div className="mt-3 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
                                    {dreamError}
                                </div>
                            )}
                        </div>

                        <div className="space-y-2">
                            {dreamProposals.length === 0 ? (
                                <div className="rounded-lg border border-[var(--color-border)] bg-black/20 p-6 text-center text-sm text-[var(--color-text-muted)]">
                                    {t("settings.memory.dream.empty_proposals")}
                                </div>
                            ) : (
                                dreamProposals.map((proposal) => (
                                    <div
                                        key={proposal.id}
                                        className="rounded-lg border border-[var(--color-border)] bg-black/20 p-3"
                                    >
                                        <div className="flex items-start justify-between gap-3">
                                            <div className="min-w-0 flex-1">
                                                <div className="flex flex-wrap items-center gap-2">
                                                    <span className="text-sm font-semibold text-[var(--color-text-primary)]">
                                                        {formatProposalTitle(proposal)}
                                                    </span>
                                                    <span className="rounded border border-[var(--color-border)] px-1.5 py-0.5 text-[10px] text-[var(--color-text-muted)]">
                                                        {translateDreamCode("proposal_types", proposal.proposal_type)}
                                                    </span>
                                                    <span className="rounded border border-[var(--color-accent)]/30 bg-[var(--color-accent)]/10 px-1.5 py-0.5 text-[10px] text-[var(--color-accent)]">
                                                        {formatConfidence(proposal.confidence)}
                                                    </span>
                                                </div>
                                                <p className="mt-2 text-xs text-[var(--color-text-muted)]">
                                                    {formatProposalRationale(proposal)}
                                                </p>
                                                {proposal.proposed_content && (
                                                    <div className="mt-2 rounded-md border border-[var(--color-border)] bg-black/25 px-3 py-2">
                                                        <div className="mb-1 text-[10px] uppercase tracking-wider text-[var(--color-text-muted)]">
                                                            {t("settings.memory.dream.proposed_content")}
                                                        </div>
                                                        <div className="text-xs text-[var(--color-text-primary)]">
                                                            {stripStructuredMemoryPrefix(proposal.proposed_content)}
                                                        </div>
                                                    </div>
                                                )}
                                                <div className="mt-2 space-y-1.5">
                                                    <div className="flex flex-wrap gap-2 text-[10px] text-[var(--color-text-muted)]">
                                                        <span>{t("settings.memory.dream.source_memories")}</span>
                                                        {proposal.proposed_entity_key && (
                                                            <span>
                                                                {t("settings.memory.dream.entity_key", {
                                                                    key: proposal.proposed_entity_key,
                                                                })}
                                                            </span>
                                                        )}
                                                    </div>
                                                    {parseProposalIdList(proposal.source_memory_ids).map((sourceId) => {
                                                        const source = proposal.source_memories.find((item) => item.id === sourceId);
                                                        return (
                                                            <div
                                                                key={sourceId}
                                                                className="rounded-md border border-[var(--color-border)] bg-black/25 px-3 py-2"
                                                            >
                                                                <div className="mb-1 flex flex-wrap items-center gap-2 text-[10px] text-[var(--color-text-muted)]">
                                                                    <span>
                                                                        {t("settings.memory.dream.source_memory_id", {
                                                                            id: sourceId,
                                                                        })}
                                                                    </span>
                                                                    {source && (
                                                                        <span>
                                                                            {translateDreamCode("statuses", source.status)}
                                                                        </span>
                                                                    )}
                                                                </div>
                                                                <div className="text-xs text-[var(--color-text-primary)]">
                                                                    {source
                                                                        ? stripStructuredMemoryPrefix(source.content)
                                                                        : t("settings.memory.dream.source_missing")}
                                                                </div>
                                                            </div>
                                                        );
                                                    })}
                                                </div>
                                                {proposal.impact && (
                                                    <div className="mt-1 text-[10px] text-[var(--color-text-muted)]">
                                                        {formatProposalImpact(proposal)}
                                                    </div>
                                                )}
                                            </div>
                                            <div className="flex shrink-0 items-center gap-1">
                                                <button
                                                    onClick={() => handleDreamProposal(proposal.id, "reject")}
                                                    disabled={dreamActionId === proposal.id}
                                                    className="p-1.5 rounded hover:bg-red-500/20 text-[var(--color-text-muted)] hover:text-red-400 disabled:opacity-50"
                                                    title={t("settings.memory.dream.actions.reject")}
                                                >
                                                    <X size={14} />
                                                </button>
                                                <button
                                                    onClick={() => handleDreamProposal(proposal.id, "approve")}
                                                    disabled={dreamActionId === proposal.id}
                                                    className="p-1.5 rounded hover:bg-[var(--color-accent)]/20 text-[var(--color-accent)] disabled:opacity-50"
                                                    title={t("settings.memory.dream.actions.approve")}
                                                >
                                                    <Check size={14} />
                                                </button>
                                            </div>
                                        </div>
                                    </div>
                                ))
                            )}
                        </div>

                        {dreamJobs.length > 0 && (
                            <div className="rounded-lg border border-[var(--color-border)] bg-black/20 p-3">
                                <div className="mb-2 text-[10px] uppercase tracking-wider text-[var(--color-text-muted)]">
                                    {t("settings.memory.dream.recent_jobs")}
                                </div>
                                <div className="space-y-1">
                                    {dreamJobs.map((job) => (
                                        <div key={job.id} className="flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--color-text-muted)]">
                                            <span>
                                                {translateDreamCode("job_triggers", job.trigger)}
                                                {" · "}
                                                {translateDreamCode("statuses", job.status)}
                                            </span>
                                            <span>{getTimeAgo(job.started_at)}</span>
                                        </div>
                                    ))}
                                </div>
                            </div>
                        )}
                    </div>
                ) : loading && memories.length === 0 ? (
                    <div className="absolute inset-0 flex items-center justify-center">
                        <div className="text-[var(--color-text-muted)] text-sm animate-pulse">{t("settings.memory.loading")}</div>
                    </div>
                ) : filtered.length === 0 ? (
                    <div className="text-center py-12">
                        <Brain
                            size={32}
                            className="mx-auto mb-3 text-[var(--color-text-muted)] opacity-30"
                        />
                        <p className="text-sm text-[var(--color-text-muted)]">
                            {searchQuery
                                ? t("settings.memory.empty.search")
                                : t("settings.memory.empty.all")}
                        </p>
                    </div>
                ) : view === "timeline" ? (
                    <MemoryTimeline
                        memories={filtered}
                        onSelect={(mem) => {
                            setView("list");
                            setSearchQuery(stripStructuredMemoryPrefix(mem.content).substring(0, 20)); // Quick hack to jump to it
                        }}
                    />
                ) : view === "graph" ? (
                    <MemoryGraph
                        memories={filtered}
                        onSelectKeyword={(kw) => {
                            setSearchQuery(kw);
                            setView("list");
                        }}
                    />
                ) : (
                    /* LIST VIEW */
                    <div className="space-y-2 pb-4">
                        <AnimatePresence mode="popLayout">
                            {filtered.map((mem) => (
                                <motion.div
                                    key={mem.id}
                                    layout
                                    initial={{ opacity: 0, y: 8 }}
                                    animate={{ opacity: 1, y: 0 }}
                                    exit={{ opacity: 0, scale: 0.95 }}
                                    className={clsx(
                                        "group rounded-lg border p-3 transition-all",
                                        editingId === mem.id
                                            ? "border-[var(--color-accent)] bg-[var(--color-accent)]/5"
                                            : "border-[var(--color-border)] bg-black/20 hover:border-[var(--color-border-hover)]"
                                    )}
                                >
                                    {editingId === mem.id ? (
                                        /* ── Edit Mode ── */
                                        <div className="space-y-3">
                                            <textarea
                                                value={editContent}
                                                onChange={(e) =>
                                                    setEditContent(e.target.value)
                                                }
                                                rows={3}
                                                className={clsx(
                                                    inputClasses,
                                                    "resize-none text-xs"
                                                )}
                                                autoFocus
                                            />
                                            <div className="flex items-center gap-3">
                                                <label className="text-[10px] font-heading uppercase tracking-wider text-[var(--color-text-muted)]">
                                                    {t("settings.memory.edit.importance")}
                                                </label>
                                                <input
                                                    type="range"
                                                    min="0"
                                                    max="1"
                                                    step="0.1"
                                                    value={editImportance}
                                                    onChange={(e) =>
                                                        setEditImportance(
                                                            parseFloat(
                                                                e.target.value
                                                            )
                                                        )
                                                    }
                                                    className="flex-1 accent-[var(--color-accent)]"
                                                />
                                                <span
                                                    className={clsx(
                                                        "text-[10px] font-mono px-1.5 py-0.5 rounded border",
                                                        getImportanceColor(editImportance)
                                                    )}
                                                >
                                                    {editImportance.toFixed(1)}
                                                </span>
                                            </div>
                                            <div className="flex justify-end gap-2">
                                                <button
                                                    onClick={() => {
                                                        setEditingId(null);
                                                        setEditContentPrefix(null);
                                                    }}
                                                    className="p-1.5 rounded hover:bg-white/5 text-[var(--color-text-muted)]"
                                                >
                                                    <X size={14} />
                                                </button>
                                                <button
                                                    onClick={saveEdit}
                                                    className="p-1.5 rounded hover:bg-[var(--color-accent)]/20 text-[var(--color-accent)]"
                                                >
                                                    <Check size={14} />
                                                </button>
                                            </div>
                                        </div>
                                    ) : deletingId === mem.id ? (
                                        /* ── Delete Confirm ── */
                                        <div className="flex items-center justify-between">
                                            <span className="text-xs text-red-400">
                                                {t("settings.memory.delete.confirm")}
                                            </span>
                                            <div className="flex gap-2">
                                                <button
                                                    onClick={() =>
                                                        setDeletingId(null)
                                                    }
                                                    className="px-2 py-1 text-[10px] rounded border border-[var(--color-border)] text-[var(--color-text-muted)] hover:bg-white/5"
                                                >
                                                    {t("common.actions.cancel")}
                                                </button>
                                                <button
                                                    onClick={() =>
                                                        confirmDelete(mem.id)
                                                    }
                                                    className="px-2 py-1 text-[10px] rounded border border-red-500/40 text-red-400 hover:bg-red-500/20"
                                                >
                                                    {t("common.actions.delete")}
                                                </button>
                                            </div>
                                        </div>
                                    ) : (
                                        /* ── View Mode ── */
                                        <div className="flex gap-3">
                                            <div className="flex-1 min-w-0">
                                                <p className="text-sm text-[var(--color-text-primary)] leading-relaxed break-words">
                                                    {stripStructuredMemoryPrefix(mem.content)}
                                                </p>
                                                <div className="flex items-center gap-2 mt-2">
                                                    <span
                                                        className={clsx(
                                                            "text-[9px] font-mono px-1.5 py-0.5 rounded border",
                                                            getImportanceColor(mem.importance)
                                                        )}
                                                    >
                                                        {getImportanceLabel(mem.importance)}
                                                    </span>
                                                    <span className="text-[10px] text-[var(--color-text-muted)]">
                                                        {getTimeAgo(mem.created_at)}
                                                    </span>
                                                </div>
                                            </div>
                                            <div className="flex flex-col gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                                                <button
                                                    onClick={() => startEdit(mem)}
                                                    className="p-1 rounded hover:bg-white/10 text-[var(--color-text-muted)] hover:text-[var(--color-accent)]"
                                                    title={t("common.actions.edit")}
                                                >
                                                    <Pencil size={12} />
                                                </button>
                                                <button
                                                    onClick={() =>
                                                        setDeletingId(mem.id)
                                                    }
                                                    className="p-1 rounded hover:bg-red-500/20 text-[var(--color-text-muted)] hover:text-red-400"
                                                    title={t("common.actions.delete")}
                                                >
                                                    <Trash2 size={12} />
                                                </button>
                                            </div>
                                        </div>
                                    )}
                                </motion.div>
                            ))}
                        </AnimatePresence>

                        {/* Load More */}
                        {hasMore && !searchQuery && (
                            <button
                                onClick={() => setPage((p) => p + 1)}
                                className="w-full py-2 text-xs text-[var(--color-text-muted)] hover:text-[var(--color-accent)] transition-colors flex items-center justify-center gap-1"
                            >
                                <ChevronDown size={12} />
                                {t("settings.memory.load_more")}
                            </button>
                        )}
                    </div>
                )}
            </div>
        </div>
    );
}
