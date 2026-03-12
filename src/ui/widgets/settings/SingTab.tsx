import { useState, useEffect, useRef, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { Music, Upload, Play, Pause, Download, Loader2, Wifi, WifiOff, ArrowUpDown, Settings2, ChevronDown, RefreshCw } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import {
    checkRvcStatus,
    listRvcModels,
    convertSinging,
    onSingingProgress,
    type RvcModelInfo,
    type SingingProgressEvent,
} from "../../../lib/kokoro-bridge";
import { useTranslation } from "react-i18next";
import { inputClasses, labelClasses } from "../../styles/settings-primitives";
import { Select } from "@/components/ui/select";

export default function SingTab() {
    const { t } = useTranslation();
    const [rvcOnline, setRvcOnline] = useState<boolean | null>(null);
    const [models, setModels] = useState<RvcModelInfo[]>([]);
    const [selectedModel, setSelectedModel] = useState("");
    const [pitchShift, setPitchShift] = useState(0);
    const [separateVocals, setSeparateVocals] = useState(true);

    const [showAdvanced, setShowAdvanced] = useState(false);
    const [f0Method, setF0Method] = useState("rmvpe");
    const [indexPath, setIndexPath] = useState("");
    const [indexRate, setIndexRate] = useState(0.75);

    const [audioPath, setAudioPath] = useState<string | null>(null);
    const [audioName, setAudioName] = useState("");

    const [converting, setConverting] = useState(false);
    const [progress, setProgress] = useState(0);
    const [progressStage, setProgressStage] = useState("");

    const [resultPath, setResultPath] = useState<string | null>(null);
    const [isPlaying, setIsPlaying] = useState(false);
    const audioRef = useRef<HTMLAudioElement | null>(null);

    const refreshStatus = useCallback(() => {
        setRvcOnline(null);
        checkRvcStatus().then(setRvcOnline).catch(() => setRvcOnline(false));
        listRvcModels().then(setModels).catch(() => setModels([]));
    }, []);

    useEffect(() => {
        refreshStatus();
    }, [refreshStatus]);

    useEffect(() => {
        let unlisten: (() => void) | undefined;
        onSingingProgress((event: SingingProgressEvent) => {
            setProgress(event.progress * 100);
            const stageLabels: Record<string, string> = {
                reading: t("sing.actions.progress.reading"),
                converting: t("sing.actions.progress.converting"),
                done: t("sing.actions.progress.done"),
            };
            setProgressStage(stageLabels[event.stage] || event.stage);
        }).then(fn => { unlisten = fn; });
        return () => { unlisten?.(); };
    }, [t]);

    const handleSelectFile = useCallback(async () => {
        const selected = await open({
            multiple: false,
            filters: [{ name: "Audio", extensions: ["mp3", "wav", "flac", "ogg", "m4a", "aac"] }],
        });
        if (selected) {
            setAudioPath(selected);
            setAudioName(selected.split(/[\\/]/).pop() || "unknown");
            setResultPath(null);
        }
    }, []);

    const handleConvert = useCallback(async () => {
        if (!audioPath) return;
        setConverting(true);
        setProgress(0);
        setProgressStage(t("sing.actions.progress.reading"));
        setResultPath(null);
        try {
            const result = await convertSinging(
                audioPath,
                selectedModel || undefined,
                pitchShift || undefined,
                separateVocals,
                f0Method !== "rmvpe" ? f0Method : undefined,
                indexPath.trim() || undefined,
                indexRate !== 0.75 ? indexRate : undefined,
            );
            setResultPath(result.output_path);
        } catch (err) {
            console.error("[Sing] Conversion failed:", err);
            setProgressStage(`Error: ${err}`);
        } finally {
            setConverting(false);
        }
    }, [audioPath, selectedModel, pitchShift, separateVocals, f0Method, indexPath, indexRate, t]);

    const togglePlayback = useCallback(() => {
        if (!resultPath) return;
        if (!audioRef.current) {
            audioRef.current = new Audio(`asset://localhost/${resultPath.replace(/\\/g, "/")}`);
            audioRef.current.onended = () => setIsPlaying(false);
        }
        if (isPlaying) {
            audioRef.current.pause();
            setIsPlaying(false);
        } else {
            audioRef.current.play();
            setIsPlaying(true);
        }
    }, [resultPath, isPlaying]);

    return (
        <div className="space-y-6">
            {/* RVC Server Status */}
            <div className={clsx(
                "flex items-center justify-between p-4 rounded-lg border",
                rvcOnline === null
                    ? "border-[var(--color-border)] bg-[var(--color-bg-elevated)]"
                    : rvcOnline
                        ? "border-emerald-500/30 bg-emerald-500/5"
                        : "border-red-500/30 bg-red-500/5"
            )}>
                <div className="flex items-center gap-3">
                    {rvcOnline
                        ? <Wifi size={16} className="text-emerald-400" />
                        : <WifiOff size={16} className={rvcOnline === null ? "text-[var(--color-text-muted)]" : "text-red-400"} />
                    }
                    <div>
                        <p className="text-sm font-heading font-bold text-[var(--color-text-primary)]">
                            {t("sing.rvc_server.title")}
                        </p>
                        <p className="text-xs text-[var(--color-text-muted)]">
                            {rvcOnline === null
                                ? t("sing.rvc_server.checking")
                                : rvcOnline
                                    ? t("sing.rvc_server.online")
                                    : t("sing.rvc_server.offline")}
                        </p>
                    </div>
                </div>
                <button
                    onClick={refreshStatus}
                    className="text-xs uppercase tracking-wider text-[var(--color-accent)] hover:underline flex items-center gap-1"
                >
                    <RefreshCw size={10} />
                    {t("sing.rvc_server.refresh")}
                </button>
            </div>

            {/* File Selection */}
            <div className="space-y-2">
                <label className={labelClasses}>{t("sing.file_selection.title")}</label>
                <button
                    onClick={handleSelectFile}
                    className={clsx(
                        "w-full py-6 rounded-lg border-2 border-dashed transition-all",
                        "flex flex-col items-center gap-2",
                        audioPath
                            ? "border-[var(--color-accent)]/40 bg-[var(--color-accent)]/5"
                            : "border-[var(--color-border)] bg-black/20 hover:border-[var(--color-accent)]/30 hover:bg-black/30"
                    )}
                >
                    <Upload size={20} className="text-[var(--color-text-muted)]" />
                    {audioPath ? (
                        <span className="text-sm text-[var(--color-accent)]">{audioName}</span>
                    ) : (
                        <span className="text-sm text-[var(--color-text-muted)]">{t("sing.file_selection.placeholder")}</span>
                    )}
                </button>
            </div>

            {/* Settings */}
            <div className="space-y-4 p-4 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-elevated)]">
                <h3 className="text-xs font-heading font-bold text-[var(--color-text-muted)] uppercase tracking-wider">
                    {t("sing.settings.model_label")}
                </h3>

                {/* Model Selection */}
                {models.length > 0 && (
                    <div>
                        <label className={labelClasses}>{t("sing.settings.model_label")}</label>
                        <Select
                            value={selectedModel}
                            onChange={setSelectedModel}
                            options={[
                                { value: "", label: t("sing.settings.default_model") },
                                ...models.map(m => ({ value: m.name, label: m.name })),
                            ]}
                        />
                    </div>
                )}

                {/* Pitch Shift */}
                <div>
                    <div className="flex items-center justify-between mb-2">
                        <div className="flex items-center gap-2">
                            <ArrowUpDown size={12} className="text-[var(--color-text-muted)]" />
                            <label className={labelClasses.replace("mb-2", "mb-0")}>{t("sing.settings.pitch_shift")}</label>
                        </div>
                        <span className="text-xs text-[var(--color-text-primary)] font-mono">
                            {pitchShift > 0 ? `+${pitchShift}` : pitchShift}
                        </span>
                    </div>
                    <input
                        type="range" min={-12} max={12} step={1}
                        value={pitchShift}
                        onChange={(e) => setPitchShift(Number(e.target.value))}
                        className="w-full accent-[var(--color-accent)]"
                    />
                    <div className="flex justify-between text-xs text-[var(--color-text-muted)] mt-1">
                        <span>-12</span><span>0</span><span>+12</span>
                    </div>
                </div>

                {/* Vocal Separation Toggle */}
                <div className="flex items-center justify-between">
                    <div>
                        <p className="text-sm text-[var(--color-text-primary)]">{t("sing.settings.separate_vocals.title")}</p>
                        <p className="text-xs text-[var(--color-text-muted)]">{t("sing.settings.separate_vocals.description")}</p>
                    </div>
                    <button
                        onClick={() => setSeparateVocals(!separateVocals)}
                        className={clsx(
                            "w-10 h-6 rounded-full transition-colors relative",
                            separateVocals ? "bg-[var(--color-accent)]" : "bg-[var(--color-border)]"
                        )}
                    >
                        <motion.div
                            animate={{ x: separateVocals ? 18 : 2 }}
                            className="absolute top-1 w-4 h-4 rounded-full bg-white shadow-sm"
                        />
                    </button>
                </div>

                {/* Advanced Settings Toggle */}
                <button
                    onClick={() => setShowAdvanced(!showAdvanced)}
                    className="w-full flex items-center justify-between py-2 text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors border-t border-[var(--color-border)] pt-3 mt-1"
                >
                    <div className="flex items-center gap-1.5">
                        <Settings2 size={12} />
                        <span className="uppercase tracking-wider font-heading">{t("sing.settings.advanced.toggle")}</span>
                    </div>
                    <ChevronDown size={12} className={clsx("transition-transform", showAdvanced && "rotate-180")} />
                </button>

                <AnimatePresence>
                    {showAdvanced && (
                        <motion.div
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: "auto", opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            transition={{ duration: 0.2 }}
                            className="overflow-hidden space-y-4"
                        >
                            <div>
                                <label className={labelClasses}>{t("sing.settings.advanced.f0_method.label")}</label>
                                <Select
                                    value={f0Method}
                                    onChange={setF0Method}
                                    options={[
                                        { value: "rmvpe", label: t("sing.settings.advanced.f0_method.rmvpe") },
                                        { value: "pm", label: t("sing.settings.advanced.f0_method.pm") },
                                        { value: "harvest", label: t("sing.settings.advanced.f0_method.harvest") },
                                        { value: "crepe", label: t("sing.settings.advanced.f0_method.crepe") },
                                    ]}
                                />
                            </div>

                            <div>
                                <label className={labelClasses}>{t("sing.settings.advanced.index_path.label")}</label>
                                <input
                                    type="text"
                                    value={indexPath}
                                    onChange={(e) => setIndexPath(e.target.value)}
                                    placeholder={t("sing.settings.advanced.index_path.placeholder")}
                                    className={inputClasses}
                                />
                            </div>

                            <div>
                                <div className="flex items-center justify-between mb-2">
                                    <label className={labelClasses.replace("mb-2", "mb-0")}>{t("sing.settings.advanced.index_rate")}</label>
                                    <span className="text-xs text-[var(--color-text-primary)] font-mono">{indexRate.toFixed(2)}</span>
                                </div>
                                <input
                                    type="range" min={0} max={1} step={0.05}
                                    value={indexRate}
                                    onChange={(e) => setIndexRate(Number(e.target.value))}
                                    className="w-full accent-[var(--color-accent)]"
                                />
                                <div className="flex justify-between text-xs text-[var(--color-text-muted)] mt-1">
                                    <span>0</span><span>0.75</span><span>1</span>
                                </div>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>

            {/* Convert Button */}
            <button
                onClick={handleConvert}
                disabled={!audioPath || converting || !rvcOnline}
                className={clsx(
                    "w-full py-3 rounded-lg text-sm font-heading font-bold uppercase tracking-wider transition-all",
                    "flex items-center justify-center gap-2 border",
                    audioPath && !converting && rvcOnline
                        ? "border-[var(--color-accent)] bg-[var(--color-accent)]/10 text-[var(--color-accent)] hover:bg-[var(--color-accent)]/20"
                        : "border-[var(--color-border)] bg-black/20 text-[var(--color-text-muted)] cursor-not-allowed"
                )}
            >
                {converting ? (
                    <><Loader2 size={14} className="animate-spin" />{progressStage}</>
                ) : (
                    <><Music size={14} />{t("sing.actions.convert")}</>
                )}
            </button>

            {/* Progress Bar */}
            {converting && (
                <div className="h-1 rounded-full bg-[var(--color-border)] overflow-hidden">
                    <motion.div
                        className="h-full bg-[var(--color-accent)] rounded-full"
                        initial={{ width: 0 }}
                        animate={{ width: `${progress}%` }}
                        transition={{ duration: 0.3 }}
                    />
                </div>
            )}

            {/* Result Playback */}
            {resultPath && !converting && (
                <motion.div
                    initial={{ opacity: 0, y: 4 }}
                    animate={{ opacity: 1, y: 0 }}
                    className="rounded-lg border border-[var(--color-accent)]/30 bg-[var(--color-accent)]/5 p-4 space-y-3"
                >
                    <p className="text-xs font-heading font-bold uppercase tracking-wider text-[var(--color-accent)]">
                        {t("sing.actions.success")}
                    </p>
                    <div className="flex gap-2">
                        <button
                            onClick={togglePlayback}
                            className="flex-1 flex items-center justify-center gap-2 py-2 rounded-lg border border-[var(--color-border)] bg-black/20 hover:bg-black/40 text-[var(--color-text-primary)] text-sm transition-colors"
                        >
                            {isPlaying
                                ? <><Pause size={14} />{t("sing.actions.pause")}</>
                                : <><Play size={14} />{t("sing.actions.play")}</>
                            }
                        </button>
                        <button
                            onClick={() => import("@tauri-apps/plugin-opener").then(m => m.openUrl(`file://${resultPath}`))}
                            className="flex items-center justify-center gap-2 px-4 py-2 rounded-lg border border-[var(--color-border)] bg-black/20 hover:bg-black/40 text-[var(--color-text-primary)] text-sm transition-colors"
                        >
                            <Download size={14} />
                        </button>
                    </div>
                </motion.div>
            )}
        </div>
    );
}
