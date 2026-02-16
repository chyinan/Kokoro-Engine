import { createContext, useContext, useEffect, useState, ReactNode } from "react";
import { ThemeConfig } from "../layout/types";

interface ThemeContextType {
    activeTheme: ThemeConfig | null;
    setTheme: (theme: ThemeConfig) => void;
}

const ThemeContext = createContext<ThemeContextType | undefined>(undefined);

export function useTheme() {
    const context = useContext(ThemeContext);
    if (!context) {
        throw new Error("useTheme must be used within a ThemeProvider");
    }
    return context;
}

export function ThemeProvider({ children, initialTheme }: { children: ReactNode; initialTheme?: ThemeConfig }) {
    const [activeTheme, setActiveTheme] = useState<ThemeConfig | null>(initialTheme || null);

    // Listen for mod theme overrides dispatched from App.tsx
    useEffect(() => {
        const handleModTheme = (event: Event) => {
            const customEvent = event as CustomEvent<ThemeConfig>;
            if (customEvent.detail) {
                console.log("[ThemeProvider] Applying mod theme:", customEvent.detail.name);
                setActiveTheme(customEvent.detail);
            }
        };
        document.addEventListener("kokoro:mod-theme", handleModTheme);
        return () => document.removeEventListener("kokoro:mod-theme", handleModTheme);
    }, []);

    useEffect(() => {
        if (!activeTheme) return;

        const root = document.documentElement;

        // 1. Inject CSS Variables
        Object.entries(activeTheme.variables).forEach(([key, value]) => {
            root.style.setProperty(key, value);
        });

        // 2. Load Fonts
        if (activeTheme.assets?.fonts) {
            activeTheme.assets.fonts.forEach((fontUrl) => {
                if (!document.querySelector(`link[href="${fontUrl}"]`)) {
                    const link = document.createElement("link");
                    link.href = fontUrl;
                    link.rel = "stylesheet";
                    document.head.appendChild(link);
                }
            });
        }

        // 3. Set Background Color
        document.body.style.backgroundColor = activeTheme.variables["--color-bg-primary"] || "#050510";

        // 4. Set Background Image (if provided)
        if (activeTheme.assets?.background) {
            document.body.style.backgroundImage = `url("${activeTheme.assets.background}")`;
            document.body.style.backgroundSize = "cover";
            document.body.style.backgroundPosition = "center";
            document.body.style.backgroundRepeat = "no-repeat";
        }

    }, [activeTheme]);

    return (
        <ThemeContext.Provider value={{ activeTheme, setTheme: setActiveTheme }}>
            {children}
        </ThemeContext.Provider>
    );
}
