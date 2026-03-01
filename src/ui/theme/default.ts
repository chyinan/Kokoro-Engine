import { ThemeConfig } from "../layout/types";

export const defaultTheme: ThemeConfig = {
    id: "neon-glass",
    name: "Neon Glass",
    variables: {
        // Colors: Backgrounds
        "--color-bg-primary": "#050510",
        "--color-bg-surface": "rgba(10, 10, 20, 0.6)",
        "--color-bg-overlay": "rgba(0, 0, 0, 0.4)",
        "--color-bg-elevated": "rgba(15, 15, 30, 0.8)",

        // Colors: Text
        "--color-text-primary": "#e0e6ed",
        "--color-text-secondary": "#94a3b8",
        "--color-text-muted": "#64748b",

        // Colors: Accent
        "--color-accent": "#00f0ff",
        "--color-accent-hover": "#33f5ff",
        "--color-accent-active": "#00d4e0",
        "--color-accent-subtle": "rgba(0, 240, 255, 0.10)",

        // Colors: Semantic
        "--color-success": "#10b981",
        "--color-warning": "#f59e0b",
        "--color-error": "#ef4444",

        // Colors: Borders
        "--color-border": "rgba(255, 255, 255, 0.08)",
        "--color-border-accent": "rgba(0, 240, 255, 0.30)",
        "--color-border-subtle": "rgba(255, 255, 255, 0.04)",

        // Glow Shadows
        "--glow-accent": "0 0 10px rgba(0, 240, 255, 0.5)",
        "--glow-accent-hover": "0 0 14px rgba(0, 240, 255, 0.6)",
        "--glow-accent-active": "0 0 6px rgba(0, 240, 255, 0.3)",
        "--glow-success": "0 0 8px rgba(16, 185, 129, 0.5)",

        // Surfaces
        "--glass-blur": "12px",
        "--radius-sm": "6px",
        "--radius-md": "8px",
        "--radius-lg": "12px",
        "--radius-xl": "16px",

        // Shadows
        "--shadow-sm": "0 2px 8px rgba(0, 0, 0, 0.2)",
        "--shadow-md": "0 4px 16px rgba(0, 0, 0, 0.25)",
        "--shadow-lg": "0 8px 32px rgba(0, 0, 0, 0.3)",

        // Typography
        "--font-heading": "'Rajdhani', sans-serif",
        "--font-body": "'Quicksand', sans-serif",
        "--font-mono": "'JetBrains Mono', 'Fira Code', 'Consolas', 'Microsoft YaHei', 'SimHei', monospace",
    },
    assets: {
        fonts: [
            "https://fonts.googleapis.com/css2?family=Rajdhani:wght@500;600;700&family=Quicksand:wght@400;500;600&family=JetBrains+Mono:wght@400;500&display=swap",
        ],
    },
    animations: {
        panelEntry: {
            initial: { opacity: 0, x: -20, scale: 0.95 },
            animate: { opacity: 1, x: 0, scale: 1 },
            exit: { opacity: 0, x: -20, scale: 0.95 },
            transition: { type: "spring", stiffness: 300, damping: 30 },
        },
        messageEntry: {
            initial: { opacity: 0, y: 10, scale: 0.95 },
            animate: { opacity: 1, y: 0, scale: 1 },
            transition: { duration: 0.3 },
        },
        modalOverlay: {
            initial: { opacity: 0 },
            animate: { opacity: 1 },
            exit: { opacity: 0 },
            transition: { duration: 0.2 },
        },
        moodShift: {
            transition: { duration: 0.5, ease: "easeInOut" },
        },
    },
};
