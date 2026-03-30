import * as React from "react";
import { cn } from "@/lib/utils";

export interface SelectOption {
    value: string;
    label: string;
    disabled?: boolean;
}

export interface SelectProps {
    value: string;
    onChange: (value: string) => void;
    options: SelectOption[];
    className?: string;
    disabled?: boolean;
    placeholder?: string;
}

const ChevronIcon = () => (
    <svg
        width="12"
        height="12"
        viewBox="0 0 12 12"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        className="pointer-events-none"
    >
        <path
            d="M2 4L6 8L10 4"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
        />
    </svg>
);

const Select = React.forwardRef<HTMLDivElement, SelectProps>(
    ({ value, onChange, options, className, disabled, placeholder }, ref) => {
        const [open, setOpen] = React.useState(false);
        const [dropUp, setDropUp] = React.useState(false);
        const containerRef = React.useRef<HTMLDivElement>(null);

        React.useImperativeHandle(ref, () => containerRef.current!);

        React.useEffect(() => {
            if (!open) return;
            const handler = (e: MouseEvent) => {
                if (!containerRef.current?.contains(e.target as Node)) {
                    setOpen(false);
                }
            };
            document.addEventListener("mousedown", handler);
            return () => document.removeEventListener("mousedown", handler);
        }, [open]);

        const handleToggle = () => {
            if (disabled) return;
            if (!open && containerRef.current) {
                const rect = containerRef.current.getBoundingClientRect();
                const spaceBelow = window.innerHeight - rect.bottom;
                setDropUp(spaceBelow < 220);
            }
            setOpen(v => !v);
        };

        const selected = options.find(o => o.value === value);
        const displayLabel = selected?.label ?? placeholder ?? "";

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
                        "bg-black/40 border border-[var(--color-border)]",
                        "text-[var(--color-text-primary)] text-sm font-body",
                        "rounded-md px-4 py-3",
                        "transition-all duration-150",
                        "focus:outline-none focus:border-[var(--color-accent)] focus:shadow-[var(--glow-accent)]",
                        open && "border-[var(--color-accent)] shadow-[var(--glow-accent)]",
                        disabled && "opacity-50 cursor-not-allowed",
                        !disabled && "cursor-pointer hover:border-[var(--color-accent)]/60"
                    )}
                >
                    <span className={cn(!selected && "text-[var(--color-text-muted)]")}>
                        {displayLabel}
                    </span>
                    <span
                        className={cn(
                            "text-[var(--color-text-muted)] transition-transform duration-150",
                            open && "rotate-180"
                        )}
                    >
                        <ChevronIcon />
                    </span>
                </button>

                {open && (
                    <div
                        className={cn(
                            "absolute z-50 w-full",
                            dropUp ? "bottom-full mb-1" : "top-full mt-1",
                            "bg-[#0a0a1a] border border-[var(--color-accent)]/30",
                            "rounded-md overflow-hidden",
                            "shadow-[0_8px_32px_rgba(0,0,0,0.6),0_0_0_1px_rgba(0,240,255,0.05)]"
                        )}
                    >
                        <ul className="max-h-52 overflow-y-auto py-1 scrollbar-thin">
                            {options.map(opt => (
                                <li
                                    key={opt.value}
                                    onClick={() => {
                                        if (!opt.disabled) {
                                            onChange(opt.value);
                                            setOpen(false);
                                        }
                                    }}
                                    className={cn(
                                        "px-4 py-2.5 text-sm font-body cursor-pointer",
                                        "transition-colors duration-100",
                                        "text-[var(--color-text-primary)]",
                                        opt.value === value
                                            ? "bg-[var(--color-accent)]/15 text-[var(--color-accent)]"
                                            : "hover:bg-white/5",
                                        opt.disabled && "opacity-40 cursor-not-allowed"
                                    )}
                                >
                                    {opt.label}
                                </li>
                            ))}
                        </ul>
                    </div>
                )}
            </div>
        );
    }
);

Select.displayName = "Select";

export { Select };
