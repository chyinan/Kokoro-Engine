import { useMemo } from "react";
import { format, isToday, isYesterday, isThisWeek, isThisYear } from "date-fns";
import { clsx } from "clsx";
import { Calendar, Star } from "lucide-react";
import type { MemoryRecord } from "../../../lib/kokoro-bridge";

interface MemoryTimelineProps {
    memories: MemoryRecord[];
    onSelect?: (memory: MemoryRecord) => void;
}

export function MemoryTimeline({ memories, onSelect }: MemoryTimelineProps) {
    // Sort descending by date
    const sorted = useMemo(() => {
        return [...memories].sort((a, b) => b.created_at - a.created_at);
    }, [memories]);

    // Group by time periods
    const groups = useMemo(() => {
        const g: Record<string, MemoryRecord[]> = {};

        sorted.forEach(mem => {
            const date = new Date(mem.created_at * 1000);
            let key = "";

            if (isToday(date)) key = "Today";
            else if (isYesterday(date)) key = "Yesterday";
            else if (isThisWeek(date)) key = "This Week";
            else if (isThisYear(date)) key = format(date, "MMMM");
            else key = format(date, "yyyy");

            if (!g[key]) g[key] = [];
            g[key].push(mem);
        });

        // Order keys manually based on recency logic above
        // Since we insert in order, Object.keys usually preserves it but let's be safe
        // Actually since source is sorted, keys will be created in order of appearance
        return g;
    }, [sorted]);

    const importanceColor = (v: number) => {
        if (v >= 8) return "bg-amber-500 shadow-amber-500/50";
        if (v >= 5) return "bg-emerald-500 shadow-emerald-500/50";
        return "bg-slate-500";
    };

    return (
        <div className="relative pl-4 py-2 space-y-8">
            {/* Vertical timeline line */}
            <div className="absolute left-6 top-4 bottom-4 w-px bg-slate-800" />

            {Object.entries(groups).map(([label, groupMems]) => (
                <div key={label} className="relative">
                    {/* Period Label */}
                    <div className="flex items-center gap-3 mb-4 sticky top-0 bg-slate-900/90 backdrop-blur z-10 py-2">
                        <div className="w-4 h-4 rounded-full bg-slate-800 border-2 border-slate-700 ml-[0.3rem]" />
                        <span className="text-xs font-bold text-slate-500 uppercase tracking-wider">{label}</span>
                    </div>

                    <div className="space-y-4 pl-8">
                        {groupMems.map(mem => (
                            <div
                                key={mem.id}
                                onClick={() => onSelect?.(mem)}
                                className="group relative bg-slate-800/40 hover:bg-slate-800/60 border border-slate-700/50 hover:border-slate-600 transition-all rounded-lg p-3 cursor-pointer"
                            >
                                {/* Connector dot */}
                                <div className={clsx(
                                    "absolute -left-[1.6rem] top-4 w-2.5 h-2.5 rounded-full border-2 border-slate-900 transition-all shadow-lg",
                                    importanceColor(mem.importance)
                                )} />

                                {/* Content */}
                                <div className="flex justify-between items-start gap-3">
                                    <p className="text-sm text-slate-200 line-clamp-3 leading-relaxed">
                                        {mem.content}
                                    </p>
                                    {mem.importance >= 8 && (
                                        <Star size={12} className="text-amber-500 fill-amber-500/20 shrink-0 mt-1" />
                                    )}
                                </div>

                                <div className="mt-2 flex items-center justify-between">
                                    <span className="text-[10px] text-slate-500 font-mono">
                                        {format(new Date(mem.created_at * 1000), "HH:mm")}
                                    </span>
                                </div>
                            </div>
                        ))}
                    </div>
                </div>
            ))}

            {memories.length === 0 && (
                <div className="text-center py-12 text-slate-500">
                    <Calendar className="mx-auto mb-2 opacity-20" size={32} />
                    <p className="text-sm">No memories recorded yet.</p>
                </div>
            )}
        </div>
    );
}
