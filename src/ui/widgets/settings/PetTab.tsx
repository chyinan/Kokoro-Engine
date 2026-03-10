import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface PetConfig {
    enabled: boolean;
    position_x: number;
    position_y: number;
    shortcut: string;
    model_url: string | null;
    window_width: number;
    window_height: number;
    model_scale: number;
}

const isMac = navigator.platform.toUpperCase().includes("MAC");

// Convert Tauri shortcut string to display string
function shortcutToDisplay(shortcut: string): string {
    return shortcut
        .replace("CmdOrCtrl", isMac ? "⌘" : "Ctrl")
        .replace("Alt", isMac ? "⌥" : "Alt")
        .replace("Shift", "Shift")
        .split("+")
        .join(" + ");
}

// Convert recorded keys to Tauri shortcut string
function keysToTauri(keys: Set<string>): string {
    const parts: string[] = [];
    if (keys.has("Meta") || keys.has("Control")) parts.push("CmdOrCtrl");
    if (keys.has("Shift")) parts.push("Shift");
    if (keys.has("Alt")) parts.push("Alt");
    // Find the main key (non-modifier)
    const modifiers = new Set(["Meta", "Control", "Shift", "Alt"]);
    for (const k of keys) {
        if (!modifiers.has(k)) {
            parts.push(k.length === 1 ? k.toUpperCase() : k);
        }
    }
    return parts.join("+");
}

export default function PetTab() {
    const [config, setConfig] = useState<PetConfig>({
        enabled: false,
        position_x: 100,
        position_y: 100,
        shortcut: "CmdOrCtrl+Shift+Space",
        model_url: null,
        window_width: 0,
        window_height: 0,
        model_scale: 0,
    });
    const [saved, setSaved] = useState(false);
    const [recording, setRecording] = useState(false);
    const [recordingDisplay, setRecordingDisplay] = useState("");

    const startRecording = useCallback(() => {
        setRecording(true);
        setRecordingDisplay("请按下快捷键...");

        const pressedKeys = new Set<string>();

        const onKeyDown = (e: KeyboardEvent) => {
            e.preventDefault();
            pressedKeys.add(e.key);
            // Build display string live
            const parts: string[] = [];
            if (pressedKeys.has("Meta")) parts.push(isMac ? "⌘" : "Win");
            if (pressedKeys.has("Control")) parts.push("Ctrl");
            if (pressedKeys.has("Shift")) parts.push("Shift");
            if (pressedKeys.has("Alt")) parts.push(isMac ? "⌥" : "Alt");
            const modifiers = new Set(["Meta", "Control", "Shift", "Alt"]);
            for (const k of pressedKeys) {
                if (!modifiers.has(k)) parts.push(k.length === 1 ? k.toUpperCase() : k);
            }
            setRecordingDisplay(parts.join(" + ") || "请按下快捷键...");
        };

        const onKeyUp = () => {
            const tauri = keysToTauri(pressedKeys);
            if (tauri) {
                setConfig(c => ({ ...c, shortcut: tauri }));
            }
            setRecording(false);
            document.removeEventListener("keydown", onKeyDown);
            document.removeEventListener("keyup", onKeyUp);
        };

        document.addEventListener("keydown", onKeyDown);
        document.addEventListener("keyup", onKeyUp);
    }, []);

    useEffect(() => {
        invoke<PetConfig>("get_pet_config").then(setConfig).catch(console.error);

        // Listen for pet window close event
        const unlisten = listen("pet-window-closed", () => {
            console.log("[PetTab] Pet window closed, updating config");
            setConfig(c => ({ ...c, enabled: false }));
            // Also save to backend
            invoke<PetConfig>("get_pet_config").then(cfg => {
                invoke("save_pet_config", { config: { ...cfg, enabled: false } }).catch(console.error);
            }).catch(console.error);
        });

        return () => {
            unlisten.then(fn => fn());
        };
    }, []);

    const handleToggle = async (enabled: boolean) => {
        const newCfg = { ...config, enabled };
        setConfig(newCfg);
        await invoke("save_pet_config", { config: newCfg }).catch(console.error);
        if (enabled) {
            await invoke("show_pet_window").catch(console.error);
        } else {
            await invoke("hide_pet_window").catch(console.error);
        }
    };

    const handleSave = async () => {
        await invoke("save_pet_config", { config }).catch(console.error);
        setSaved(true);
        setTimeout(() => setSaved(false), 2000);
    };

    const handleResetPosition = async () => {
        await invoke("move_pet_window", { x: 100, y: 100 }).catch(console.error);
        const newCfg = { ...config, position_x: 100, position_y: 100 };
        setConfig(newCfg);
        await invoke("save_pet_config", { config: newCfg }).catch(console.error);
    };

    const labelStyle: React.CSSProperties = {
        fontSize: "12px",
        color: "var(--color-text-secondary)",
        textTransform: "uppercase",
        letterSpacing: "0.08em",
        fontWeight: 600,
        marginBottom: "6px",
    };

    const inputStyle: React.CSSProperties = {
        width: "100%",
        background: "var(--color-bg-surface-soft)",
        border: "1px solid var(--color-border)",
        borderRadius: "8px",
        padding: "8px 12px",
        color: "var(--color-text-primary)",
        fontSize: "13px",
        outline: "none",
    };

    return (
        <div style={{ display: "flex", flexDirection: "column", gap: "20px" }}>
            {/* Enable toggle */}
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
                <div>
                    <div style={{ fontSize: "14px", fontWeight: 600, color: "var(--color-text-primary)" }}>
                        桌面悬浮模式
                    </div>
                    <div style={{ fontSize: "12px", color: "var(--color-text-muted)", marginTop: "2px" }}>
                        在桌面显示透明悬浮 Live2D 模型
                    </div>
                </div>
                <button
                    onClick={() => handleToggle(!config.enabled)}
                    style={{
                        width: "44px", height: "24px", borderRadius: "12px", border: "none",
                        background: config.enabled ? "var(--color-accent)" : "var(--color-border)",
                        cursor: "pointer", position: "relative", transition: "background 0.2s",
                    }}
                >
                    <div style={{
                        position: "absolute", top: "3px",
                        left: config.enabled ? "23px" : "3px",
                        width: "18px", height: "18px", borderRadius: "50%",
                        background: "#fff", transition: "left 0.2s",
                    }} />
                </button>
            </div>

            {/* Shortcut */}
            <div>
                <div style={labelStyle}>全局快捷键</div>
                <div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
                    <div style={{
                        flex: 1, ...inputStyle,
                        display: "flex", alignItems: "center",
                        color: recording ? "var(--color-accent)" : "var(--color-text-primary)",
                        fontFamily: "monospace",
                        minHeight: "36px",
                    }}>
                        {recording ? recordingDisplay : shortcutToDisplay(config.shortcut)}
                    </div>
                    <button
                        onClick={startRecording}
                        disabled={recording}
                        style={{
                            padding: "8px 14px", borderRadius: "8px",
                            border: "1px solid var(--color-border)",
                            background: recording ? "var(--color-accent)" : "var(--color-bg-surface-soft)",
                            color: recording ? "#fff" : "var(--color-text-secondary)",
                            fontSize: "12px", cursor: recording ? "default" : "pointer",
                            whiteSpace: "nowrap",
                        }}
                    >
                        {recording ? "录制中..." : "录制"}
                    </button>
                </div>
                <div style={{ fontSize: "11px", color: "var(--color-text-muted)", marginTop: "4px" }}>
                    修改后需重启应用生效
                </div>
            </div>

            {/* Reset position */}
            <div>
                <div style={labelStyle}>窗口位置</div>
                <button
                    onClick={handleResetPosition}
                    style={{
                        padding: "8px 16px", borderRadius: "8px", border: "1px solid var(--color-border)",
                        background: "var(--color-bg-surface-soft)", color: "var(--color-text-secondary)",
                        fontSize: "13px", cursor: "pointer",
                    }}
                >
                    重置到屏幕左上角
                </button>
            </div>

            {/* Save */}
            <button
                onClick={handleSave}
                style={{
                    padding: "10px", borderRadius: "8px", border: "none",
                    background: saved ? "var(--color-success, #4caf50)" : "var(--color-accent)",
                    color: "#fff", fontSize: "13px", fontWeight: 600, cursor: "pointer",
                    transition: "background 0.2s",
                }}
            >
                {saved ? "已保存" : "保存设置"}
            </button>
        </div>
    );
}
