import { useState, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { motion, AnimatePresence } from "framer-motion";

function LoadingDots() {
    const [count, setCount] = useState(1);
    useEffect(() => {
        const id = setInterval(() => setCount(c => c >= 3 ? 1 : c + 1), 500);
        return () => clearInterval(id);
    }, []);
    return (
        <span>
            {".".repeat(count)}
            <span style={{ opacity: 0 }}>{".".repeat(3 - count)}</span>
        </span>
    );
}

export default function BubbleWindow() {
    const [text, setText] = useState("");
    const [visible, setVisible] = useState(false);
    const scrollRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        const unlisten = listen<string>("bubble-text-update", (event) => {
            setText(event.payload);
            setVisible(true);
        });
        return () => { unlisten.then(fn => fn()); };
    }, []);

    // Auto-scroll to bottom as text streams in
    useEffect(() => {
        if (scrollRef.current) {
            scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
        }
    }, [text]);

    return (
        <div style={{
            width: "100vw",
            height: "100vh",
            display: "flex",
            alignItems: "flex-end",
            justifyContent: "center",
            // 底部留出 tail 高度 + 间距，避免 tail 被裁切
            paddingBottom: "20px",
            background: "transparent",
            pointerEvents: "none",
        }}>
            <AnimatePresence>
                {visible && text && (
                    <motion.div
                        initial={{ opacity: 0, y: 8, scale: 0.95 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: 8, scale: 0.95 }}
                        transition={{ duration: 0.2 }}
                        style={{
                            position: "relative",
                            maxWidth: "290px",
                            width: "100%",
                            background: "rgba(15, 15, 25, 0.92)",
                            color: "#fff",
                            borderRadius: "16px",
                            fontSize: "13px",
                            lineHeight: "1.6",
                            backdropFilter: "blur(12px)",
                            border: "1px solid rgba(255,255,255,0.15)",
                            // 去掉 boxShadow，避免透明窗口上出现多余阴影
                            overflow: "visible",
                            pointerEvents: "auto",
                        }}
                    >
                        {/* Header */}
                        <div style={{
                            padding: "8px 14px 4px",
                            fontSize: "11px",
                            color: "rgba(255,255,255,0.4)",
                            borderBottom: "1px solid rgba(255,255,255,0.08)",
                            borderRadius: "16px 16px 0 0",
                        }}>
                            Kokoro
                        </div>

                        {/* Scrollable content */}
                        <div
                            ref={scrollRef}
                            style={{
                                padding: "8px 14px 12px",
                                maxHeight: "160px",
                                overflowY: text === "..." ? "hidden" : "auto",
                                wordBreak: "break-word",
                                borderRadius: "0 0 16px 16px",
                                scrollbarWidth: "thin",
                                scrollbarColor: "rgba(255,255,255,0.2) transparent",
                            }}
                        >
                            {text === "..." ? <LoadingDots /> : text}
                        </div>

                        {/* Tail pointing down toward pet window */}
                        <div style={{
                            position: "absolute",
                            bottom: "-8px",
                            left: "50%",
                            transform: "translateX(-50%)",
                            width: 0,
                            height: 0,
                            borderLeft: "8px solid transparent",
                            borderRight: "8px solid transparent",
                            borderTop: "8px solid rgba(15, 15, 25, 0.92)",
                            // 确保 tail 在气泡圆角之外渲染
                            zIndex: 1,
                        }} />
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}
