import { useState, useEffect, useRef, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { clsx } from "clsx";
import { Music, Upload, Play, Pause, Download, Loader2, Wifi, WifiOff, ArrowUpDown, Settings2, ChevronDown } from "lucide-react";
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

export default function SingTab() {
    const { t } = useTranslation();
    const [rvcOnline, setRvcOnline] = useState<boolean | null>(null);
    const [models, setModels] = useState<RvcModelInfo[]>([]);
    const [selectedModel, setSelectedModel] = useState("");
    const [pitchShift, setPitchShift] = useState(0);
    const [separateVocals, setSeparateVocals] = useState(true);

    // Advanced settings
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

    // Check RVC status on mount
    useEffect(() => {
        checkRvcStatus().then(setRvcOnline).catch(() => setRvcOnline(false));
        listRvcModels().then(setModels).catch(() => setModels([]));
    }, []);

    // Listen for progress events
    useEffect(() => {
        let unlisten: (() => void) | undefined;
        onSingingProgress((event: SingingProgressEvent) => {
            setProgress(event.progress * 100);
            const stageLabels: Record<string, string> = {
                reading: t("sing.actions.progress.reading"), // "读取音频..."
                converting: t("sing.actions.progress.converting"), // "转换中..."
                done: t("sing.actions.progress.done"), // "完成！"
            };
            setProgressStage(stageLabels[event.stage] || event.stage);
        }).then(fn => { unlisten = fn; });
        return () => { unlisten?.(); };
    }, [t]);

    const handleSelectFile = useCallback(async () => {
        const selected = await open({
            multiple: false,
            filters: [{
                name: "Audio",
                extensions: ["mp3", "wav", "flac", "ogg", "m4a", "aac"],
            }],
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
        setProgressStage(t("sing.actions.progress.reading")); // initially preparation/reading
        setResultPath(null);

        try {
            const result = await convertSinging(
                audioPath,
                selectedModel || undefined,
                pitchShift || undefined,
                separateVocals,
                // Pass advanced params only if user has customized them
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
        <div className="space-y-5">
            {/* RVC Server Status */}
            <motion.div
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.05 }}
                className={clsx(
                    "rounded-xl p-4 flex items-center gap-3",
                    rvcOnline === null
                        ? "bg-white/5 border border-white/10"
                        : rvcOnline
                            ? "bg-emerald-500/10 border border-emerald-500/20"
                            : "bg-red-500/10 border border-red-500/20"
                )}
            >
                {rvcOnline ? (
                    <Wifi className="h-5 w-5 text-emerald-400" />
                ) : (
                    <WifiOff className="h-5 w-5 text-red-400" />
                )}
                <div className="flex-1">
                    <p className="text-sm font-medium text-white/90">
                        {t("sing.rvc_server.title")}
                    </p>
                    <p className="text-xs text-white/50">
                        {rvcOnline === null
                            ? t("sing.rvc_server.checking")
                            : rvcOnline
                                ? t("sing.rvc_server.online")
                                : t("sing.rvc_server.offline")}
                    </p>
                </div>
                <button
                    onClick={() => {
                        setRvcOnline(null);
                        checkRvcStatus().then(setRvcOnline).catch(() => setRvcOnline(false));
                        listRvcModels().then(setModels).catch(() => setModels([]));
                    }}
                    className="text-xs text-white/40 hover:text-white/70 px-2 py-1 rounded-lg hover:bg-white/5 transition-colors"
                >
                    {t("sing.rvc_server.refresh")}
                </button>
            </motion.div>

            {/* File Selection */}
            <motion.div
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.1 }}
                className="rounded-xl bg-white/5 border border-white/10 p-4 space-y-3"
            >
                <div className="flex items-center gap-2 mb-3">
                    <Music className="h-4 w-4 text-purple-400" />
                    <span className="text-sm font-medium text-white/90">{t("sing.file_selection.title")}</span>
                </div>

                <button
                    onClick={handleSelectFile}
                    className={clsx(
                        "w-full py-6 rounded-lg border-2 border-dashed transition-all",
                        "flex flex-col items-center gap-2",
                        audioPath
                            ? "border-purple-500/30 bg-purple-500/5"
                            : "border-white/15 bg-white/3 hover:border-white/25 hover:bg-white/5"
                    )}
                >
                    <Upload className="h-6 w-6 text-white/40" />
                    {audioPath ? (
                        <span className="text-sm text-purple-300">{audioName}</span>
                    ) : (
                        <span className="text-sm text-white/40">{t("sing.file_selection.placeholder")}</span>
                    )}
                </button>
            </motion.div>

            {/* Settings */}
            <motion.div
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.15 }}
                className="rounded-xl bg-white/5 border border-white/10 p-4 space-y-4"
            >
                {/* Model Selection */}
                {models.length > 0 && (
                    <div>
                        <label className="text-xs text-white/50 block mb-1.5">{t("sing.settings.model_label")}</label>
                        <select
                            value={selectedModel}
                            onChange={(e) => setSelectedModel(e.target.value)}
                            className="w-full bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-sm text-white/80 focus:outline-none focus:border-purple-400/50"
                        >
                            <option value="">{t("sing.settings.default_model")}</option>
                            {models.map(m => (
                                <option key={m.name} value={m.name}>{m.name}</option>
                            ))}
                        </select>
                    </div>
                )}

                {/* Pitch Shift */}
                <div>
                    <div className="flex items-center justify-between mb-1.5">
                        <div className="flex items-center gap-2">
                            <ArrowUpDown className="h-3.5 w-3.5 text-white/40" />
                            <label className="text-xs text-white/50">{t("sing.settings.pitch_shift")}</label>
                        </div>
                        <span className="text-xs text-white/70 font-mono">
                            {pitchShift > 0 ? `+${pitchShift}` : pitchShift}
                        </span>
                    </div>
                    <input
                        type="range"
                        min={-12}
                        max={12}
                        step={1}
                        value={pitchShift}
                        onChange={(e) => setPitchShift(Number(e.target.value))}
                        className="w-full accent-purple-400"
                    />
                    <div className="flex justify-between text-xs text-white/30 mt-1">
                        <span>-12</span>
                        <span>0</span>
                        <span>+12</span>
                    </div>
                </div>

                {/* Vocal Separation Toggle */}
                <div className="flex items-center justify-between">
                    <div>
                        <p className="text-sm text-white/70">{t("sing.settings.separate_vocals.title")}</p>
                        <p className="text-xs text-white/40">{t("sing.settings.separate_vocals.description")}</p>
                    </div>
                    <button
                        onClick={() => setSeparateVocals(!separateVocals)}
                        className={clsx(
                            "w-10 h-6 rounded-full transition-colors relative",
                            separateVocals ? "bg-purple-500" : "bg-white/15"
                        )}
                    >
                        <div
                            className={clsx(
                                "w-4 h-4 rounded-full bg-white absolute top-1 transition-all",
                                separateVocals ? "left-5" : "left-1"
                            )}
                        />
                    </button>
                </div>

                {/* Advanced Settings Toggle */}
                <button
                    onClick={() => setShowAdvanced(!showAdvanced)}
                    className="w-full flex items-center justify-between py-2 text-xs text-white/40 hover:text-white/60 transition-colors"
                >
                    <div className="flex items-center gap-1.5">
                        <Settings2 className="h-3.5 w-3.5" />
                        <span>{t("sing.settings.advanced.toggle")}</span>
                    </div>
                    <ChevronDown className={clsx(
                        "h-3.5 w-3.5 transition-transform",
                        showAdvanced && "rotate-180"
                    )} />
                </button>

                {/* Advanced Settings Panel */}
                <AnimatePresence>
                    {showAdvanced && (
                        <motion.div
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: "auto", opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            transition={{ duration: 0.2 }}
                            className="overflow-hidden space-y-4"
                        >
                            {/* F0 Method */}
                            <div>
                                <label className="text-xs text-white/50 block mb-1.5">{t("sing.settings.advanced.f0_method.label")}</label>
                                <select
                                    value={f0Method}
                                    onChange={(e) => setF0Method(e.target.value)}
                                    className="w-full bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-sm text-white/80 focus:outline-none focus:border-purple-400/50"
                                >
                                    <option value="rmvpe">{t("sing.settings.advanced.f0_method.rmvpe")}</option>
                                    <option value="pm">{t("sing.settings.advanced.f0_method.pm")}</option>
                                    <option value="harvest">{t("sing.settings.advanced.f0_method.harvest")}</option>
                                    <option value="crepe">{t("sing.settings.advanced.f0_method.crepe")}</option>
                                </select>
                            </div>

                            {/* Index File Path */}
                            <div>
                                <label className="text-xs text-white/50 block mb-1.5">{t("sing.settings.advanced.index_path.label")}</label>
                                <input
                                    type="text"
                                    value={indexPath}
                                    onChange={(e) => setIndexPath(e.target.value)}
                                    placeholder={t("sing.settings.advanced.index_path.placeholder")}
                                    className="w-full bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-sm text-white/80 placeholder:text-white/25 focus:outline-none focus:border-purple-400/50"
                                />
                            </div>

                            {/* Index Rate */}
                            <div>
                                <div className="flex items-center justify-between mb-1.5">
                                    <label className="text-xs text-white/50">{t("sing.settings.advanced.index_rate")}</label>
                                    <span className="text-xs text-white/70 font-mono">{indexRate.toFixed(2)}</span>
                                </div>
                                <input
                                    type="range"
                                    min={0}
                                    max={1}
                                    step={0.05}
                                    value={indexRate}
                                    onChange={(e) => setIndexRate(Number(e.target.value))}
                                    className="w-full accent-purple-400"
                                />
                                <div className="flex justify-between text-xs text-white/30 mt-1">
                                    <span>0</span>
                                    <span>0.75</span>
                                    <span>1</span>
                                </div>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </motion.div>

            {/* Convert Button + Progress */}
            <motion.div
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.2 }}
            >
                <button
                    onClick={handleConvert}
                    disabled={!audioPath || converting || !rvcOnline}
                    className={clsx(
                        "w-full py-3 rounded-xl text-sm font-medium transition-all",
                        "flex items-center justify-center gap-2",
                        audioPath && !converting && rvcOnline
                            ? "bg-gradient-to-r from-purple-500 to-pink-500 text-white hover:from-purple-600 hover:to-pink-600 shadow-lg shadow-purple-500/25"
                            : "bg-white/10 text-white/30 cursor-not-allowed"
                    )}
                >
                    {converting ? (
                        <>
                            <Loader2 className="h-4 w-4 animate-spin" />
                            {progressStage}
                        </>
                    ) : (
                        <>
                            <Music className="h-4 w-4" />
                            {t("sing.actions.convert")}
                        </>
                    )}
                </button>

                {/* Progress Bar */}
                {converting && (
                    <div className="mt-3 h-1.5 rounded-full bg-white/10 overflow-hidden">
                        <motion.div
                            className="h-full bg-gradient-to-r from-purple-500 to-pink-500 rounded-full"
                            initial={{ width: 0 }}
                            animate={{ width: `${progress}%` }}
                            transition={{ duration: 0.3 }}
                        />
                    </div>
                )}
            </motion.div>

            {/* Result Playback */}
            {resultPath && !converting && (
                <motion.div
                    initial={{ opacity: 0, scale: 0.95 }}
                    animate={{ opacity: 1, scale: 1 }}
                    className="rounded-xl bg-gradient-to-br from-purple-500/10 to-pink-500/10 border border-purple-500/20 p-4"
                >
                    <p className="text-sm text-white/70 mb-3">🎉 {t("sing.actions.success")}</p>
                    <div className="flex gap-2">
                        <button
                            onClick={togglePlayback}
                            className="flex-1 flex items-center justify-center gap-2 py-2.5 rounded-lg bg-white/10 hover:bg-white/15 text-white/80 text-sm transition-colors"
                        >
                            {isPlaying ? (
                                <><Pause className="h-4 w-4" /> {t("sing.actions.pause")}</>
                            ) : (
                                <><Play className="h-4 w-4" /> {t("sing.actions.play")}</>
                            )}
                        </button>
                        <button
                            onClick={() => {
                                // Open file in system default app
                                import("@tauri-apps/plugin-opener").then(m => m.openUrl(`file://${resultPath}`));
                            }}
                            className="flex items-center justify-center gap-2 px-4 py-2.5 rounded-lg bg-white/10 hover:bg-white/15 text-white/80 text-sm transition-colors"
                        >
                            <Download className="h-4 w-4" />
                        </button>
                    </div>
                </motion.div>
            )}
        </div>
    );
}
