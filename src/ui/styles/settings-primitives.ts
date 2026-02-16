/**
 * Shared styling primitives for Settings sub-panels.
 *
 * Centralises the input / label / toggle classes that were previously
 * duplicated across SettingsPanel, ImageGenSettings, and MemoryPanel.
 */
import { clsx } from "clsx";

// ── Text input / select base classes ───────────────────
export const inputClasses = clsx(
    "w-full bg-black/40 border border-[var(--color-border)]",
    "text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)]",
    "rounded-md px-4 py-3 text-sm",
    "focus:outline-none focus:border-[var(--color-accent)] focus:shadow-[var(--glow-accent)]",
    "transition-all font-body"
);

// ── Section label ──────────────────────────────────────
export const labelClasses =
    "block text-xs font-heading font-semibold tracking-wider uppercase text-[var(--color-text-secondary)] mb-2";

// ── Section heading (ACTIVE SETTINGS, MANAGE PROVIDERS) ─
export const sectionHeadingClasses =
    "text-xs font-heading font-bold text-[var(--color-text-muted)] uppercase tracking-wider";
