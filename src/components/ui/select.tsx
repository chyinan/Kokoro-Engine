import * as React from "react";
import { createPortal } from "react-dom";
import { cn } from "@/lib/utils";
import { Search, ChevronDown, Check, X } from "lucide-react";

export interface SelectOption {
    value: string;
    label: string;
    disabled?: boolean;
    description?: string;
}

export interface SelectGroup {
    label: string;
    options: SelectOption[];
}

export interface SelectProps {
    value: string;
    onChange: (value: string) => void;
    options: SelectOption[] | SelectGroup[];
    className?: string;
    disabled?: boolean;
    placeholder?: string;
    searchable?: boolean;
    searchPlaceholder?: string;
    emptyMessage?: string;
}

const Select = React.forwardRef<HTMLDivElement, SelectProps>(
    (
        {
            value,
            onChange,
            options,
            className,
            disabled,
            placeholder,
            searchable = false,
            searchPlaceholder = "Search...",
            emptyMessage = "No options found",
        },
        ref
    ) => {
        const [open, setOpen] = React.useState(false);
        const [dropUp, setDropUp] = React.useState(false);
        const [searchQuery, setSearchQuery] = React.useState("");
        const [coords, setCoords] = React.useState({ top: 0, left: 0, width: 0 });
        const containerRef = React.useRef<HTMLDivElement>(null);
        const searchInputRef = React.useRef<HTMLInputElement>(null);

        React.useImperativeHandle(ref, () => containerRef.current!);

        const updateCoords = React.useCallback(() => {
            if (containerRef.current) {
                const rect = containerRef.current.getBoundingClientRect();
                const spaceBelow = window.innerHeight - rect.bottom;
                const shouldDropUp = spaceBelow < 300;
                setDropUp(shouldDropUp);
                setCoords({
                    top: shouldDropUp ? rect.top : rect.bottom,
                    left: rect.left,
                    width: rect.width,
                });
            }
        }, []);

        React.useEffect(() => {
            if (!open) {
                setSearchQuery("");
                return;
            }

            updateCoords();

            if (!searchable) return;

            const timer = window.setTimeout(() => {
                searchInputRef.current?.focus();
            }, 50);

            return () => window.clearTimeout(timer);
        }, [open, searchable, updateCoords]);

        React.useEffect(() => {
            if (!open) return;

            const handler = (e: MouseEvent) => {
                if (!containerRef.current?.contains(e.target as Node)) {
                    const portal = document.getElementById("select-portal-root");
                    if (!portal?.contains(e.target as Node)) {
                        setOpen(false);
                    }
                }
            };

            const scrollHandler = () => {
                updateCoords();
            };

            document.addEventListener("mousedown", handler);
            window.addEventListener("scroll", scrollHandler, true);
            window.addEventListener("resize", scrollHandler);

            return () => {
                document.removeEventListener("mousedown", handler);
                window.removeEventListener("scroll", scrollHandler, true);
                window.removeEventListener("resize", scrollHandler);
            };
        }, [open, updateCoords]);

        const handleToggle = () => {
            if (disabled) return;
            // Compute coords synchronously *before* opening so the portal's first
            // render already has the correct position.  Without this, `coords`
            // starts at {0,0} and gets updated only after the effect runs on the
            // next tick – causing the dropdown to briefly appear at the top-left
            // and triggering the entry animation from there.  On macOS WebKit the
            // GPU compositor renders that stale frame visibly; Windows/Linux
            // WebView2 tends to swallow it.
            if (!open && containerRef.current) {
                const rect = containerRef.current.getBoundingClientRect();
                const spaceBelow = window.innerHeight - rect.bottom;
                const shouldDropUp = spaceBelow < 300;
                setDropUp(shouldDropUp);
                setCoords({
                    top: shouldDropUp ? rect.top : rect.bottom,
                    left: rect.left,
                    width: rect.width,
                });
            }
            setOpen(v => !v);
        };

        const isGrouped = (opts: SelectOption[] | SelectGroup[]): opts is SelectGroup[] => {
            return opts.length > 0 && 'options' in opts[0];
        };

        const flatOptions = isGrouped(options) 
            ? options.flatMap(g => g.options.map(o => ({ ...o, groupLabel: g.label })))
            : options;

        const selected = flatOptions.find(o => o.value === value);
        const displayLabel = selected?.label ?? placeholder ?? "";
        const normalizedQuery = searchQuery.trim().toLowerCase();

        const filteredGroups = React.useMemo(() => {
            if (!searchable || !normalizedQuery) {
                return isGrouped(options) ? options : [{ label: "", options }];
            }

            if (isGrouped(options)) {
                return options
                    .map(group => ({
                        ...group,
                        options: group.options.filter(opt =>
                            opt.label.toLowerCase().includes(normalizedQuery) ||
                            opt.value.toLowerCase().includes(normalizedQuery) ||
                            (opt.description && opt.description.toLowerCase().includes(normalizedQuery))
                        )
                    }))
                    .filter(group => group.options.length > 0);
            } else {
                return [{
                    label: "",
                    options: (options as SelectOption[]).filter(opt =>
                        opt.label.toLowerCase().includes(normalizedQuery) ||
                        opt.value.toLowerCase().includes(normalizedQuery) ||
                        (opt.description && opt.description.toLowerCase().includes(normalizedQuery))
                    )
                }];
            }
        }, [options, searchable, normalizedQuery]);

        const totalFilteredCount = filteredGroups.reduce((acc, g) => acc + g.options.length, 0);

        const dropdownContent = (
            <div
                id="select-portal-root"
                style={{
                    position: "fixed",
                    top: dropUp ? "auto" : `${coords.top + 8}px`,
                    bottom: dropUp ? `${window.innerHeight - coords.top + 8}px` : "auto",
                    left: `${coords.left}px`,
                    minWidth: `${coords.width}px`,
                    width: "max-content",
                    maxWidth: "320px",
                    maxHeight: "320px",
                }}
                className={cn(
                    "z-[9999] overflow-hidden sticky-dropdown",
                    "bg-[var(--color-bg-elevated)] border border-white/10",
                    "rounded-xl leading-normal",
                    "shadow-[0_12px_40px_rgba(0,0,0,0.5),0_0_0_1px_rgba(255,255,255,0.05)]",
                    "animate-in fade-in zoom-in-95 duration-200"
                )}
            >
                {searchable && (
                    <div className="sticky top-0 z-10 bg-[var(--color-bg-elevated)] border-b border-white/5 p-2">
                        <div className="relative group">
                            <Search 
                                size={14} 
                                className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--color-text-muted)] group-focus-within:text-[var(--color-accent)] transition-colors" 
                            />
                            <input
                                ref={searchInputRef}
                                type="text"
                                value={searchQuery}
                                onChange={(e) => setSearchQuery(e.target.value)}
                                placeholder={searchPlaceholder}
                                className={cn(
                                    "w-full rounded-lg bg-white/5 pl-9 pr-8 py-2",
                                    "text-xs text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]/60",
                                    "border border-white/5 focus:border-[var(--color-accent)]/50 focus:outline-none",
                                    "transition-all"
                                )}
                            />
                            {searchQuery && (
                                <button
                                    onClick={() => setSearchQuery("")}
                                    className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-[var(--color-text-muted)] hover:text-white transition-colors"
                                >
                                    <X size={12} />
                                </button>
                            )}
                        </div>
                    </div>
                )}

                <div className="max-h-[300px] overflow-y-auto py-1 scrollbar-thin">
                    {totalFilteredCount === 0 ? (
                        <div className="px-4 py-8 text-center">
                            <p className="text-xs text-[var(--color-text-muted)]">
                                {emptyMessage}
                            </p>
                        </div>
                    ) : (
                        filteredGroups.map((group, gIdx) => (
                            <React.Fragment key={group.label || gIdx}>
                                {group.label && (
                                    <div className="px-3 py-1.5 mt-1 text-[10px] font-bold text-[var(--color-text-muted)] uppercase tracking-widest bg-white/[0.02]">
                                        {group.label}
                                    </div>
                                )}
                                {group.options.map(opt => (
                                    <div
                                        key={opt.value}
                                        onClick={() => {
                                            if (!opt.disabled) {
                                                onChange(opt.value);
                                                setOpen(false);
                                            }
                                        }}
                                        className={cn(
                                            "mx-1 px-3 py-2 rounded-lg text-sm cursor-pointer",
                                            "flex items-center justify-between gap-2",
                                            "transition-all duration-150",
                                            opt.value === value
                                                ? "bg-[var(--color-accent)]/10 text-[var(--color-accent)] font-medium"
                                                : "text-[var(--color-text-primary)] hover:bg-white/5",
                                            opt.disabled && "opacity-40 cursor-not-allowed"
                                        )}
                                    >
                                        <div className="flex flex-col min-w-0">
                                            <span className="truncate">{opt.label}</span>
                                            {opt.description && (
                                                <span className="text-[10px] text-[var(--color-text-muted)] truncate">
                                                    {opt.description}
                                                </span>
                                            )}
                                        </div>
                                        {opt.value === value && (
                                            <Check size={14} className="shrink-0" />
                                        )}
                                    </div>
                                ))}
                            </React.Fragment>
                        ))
                    )}
                </div>
            </div>
        );

        return (
            <div
                ref={containerRef}
                className={cn("relative w-full", className)}
            >
                <button
                    type="button"
                    disabled={disabled}
                    onClick={handleToggle}
                    className={cn(
                        "w-full flex items-center justify-between gap-2",
                        "bg-white/5 border border-[var(--color-border)]",
                        "text-[var(--color-text-primary)] text-sm",
                        "rounded-lg px-3 py-2.5",
                        "transition-all duration-200",
                        "hover:bg-white/10 hover:border-white/20",
                        "focus:outline-none focus:ring-1 focus:ring-[var(--color-accent)]/50",
                        open && "border-[var(--color-accent)]/50 bg-white/10",
                        disabled && "opacity-50 cursor-not-allowed",
                        !disabled && "cursor-pointer"
                    )}
                >
                    <span className={cn("min-w-0 flex-1 truncate text-left", !selected && "text-[var(--color-text-muted)]")}>
                        {displayLabel}
                    </span>
                    <ChevronDown 
                        size={14} 
                        className={cn("text-[var(--color-text-muted)] transition-transform duration-200", open && "rotate-180")} 
                    />
                </button>

                {open && createPortal(dropdownContent, document.body)}
            </div>
        );
    }
);

Select.displayName = "Select";

export { Select };
