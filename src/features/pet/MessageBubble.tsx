import { useEffect, useRef, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";

interface MessageBubbleProps {
    text: string;
    visible: boolean;
}

export default function MessageBubble({ text, visible }: MessageBubbleProps) {
    const [displayed, setDisplayed] = useState("");
    const [expanded, setExpanded] = useState(false);
    const scrollRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        if (!visible || !text) {
            setDisplayed("");
            setExpanded(false);
            return;
        }
        setDisplayed("");
        let i = 0;
        const interval = setInterval(() => {
            if (i < text.length) {
                setDisplayed(text.slice(0, i + 1));
                i++;
            } else {
                clearInterval(interval);
            }
        }, 20);
        return () => clearInterval(interval);
    }, [text, visible]);

    // Auto-scroll to bottom as text streams in
    useEffect(() => {
        if (scrollRef.current) {
            scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
        }
    }, [displayed]);

    const isLong = text.length > 80;

    return (
        <AnimatePresence>
            {visible && displayed && (
                <motion.div
                    initial={{ opacity: 0, y: 8, scale: 0.95 }}
                    animate={{ opacity: 1, y: 0, scale: 1 }}
                    exit={{ opacity: 0, y: -8, scale: 0.95 }}
                    transition={{ duration: 0.2 }}
                    style={{
                        position: "fixed",
                        bottom: "calc(100vh - 60px)",
                        left: "50%",
                        transform: "translateX(-50%)",
                        width: isLong ? "320px" : "auto",
                        maxWidth: "320px",
                        minWidth: "80px",
                        background: "rgba(15, 15, 25, 0.92)",
                        color: "#fff",
                        borderRadius: "14px",
                        padding: "0",
                        fontSize: "13px",
                        lineHeight: "1.6",
                        backdropFilter: "blur(12px)",
                        border: "1px solid rgba(255,255,255,0.15)",
                        boxShadow: "0 8px 32px rgba(0,0,0,0.5)",
                        pointerEvents: "auto",
                        zIndex: 1000,
                        overflow: "hidden",
                    }}
                    onClick={() => setExpanded(e => !e)}
                >
                    {/* Header */}
                    <div style={{
                        padding: "8px 14px 6px",
                        fontSize: "11px",
                        color: "rgba(255,255,255,0.4)",
                        borderBottom: isLong ? "1px solid rgba(255,255,255,0.08)" : "none",
                        display: "flex",
                        justifyContent: "space-between",
                        alignItems: "center",
                    }}>
                        <span>Kokoro</span>
                        {isLong && (
                            <span style={{ fontSize: "10px", opacity: 0.5 }}>
                                {expanded ? "收起 ▲" : "展开 ▼"}
                            </span>
                        )}
                    </div>

                    {/* Content */}
                    <div
                        ref={scrollRef}
                        style={{
                            padding: "6px 14px 12px",
                            maxHeight: expanded ? "300px" : isLong ? "80px" : "none",
                            overflowY: expanded || isLong ? "auto" : "visible",
                            wordBreak: "break-word",
                            maskImage: !expanded && isLong
                                ? "linear-gradient(to bottom, black 60%, transparent 100%)"
                                : "none",
                        }}
                    >
                        {displayed}
                    </div>

                    {/* Tail */}
                    <div style={{
                        position: "absolute",
                        bottom: "-7px",
                        left: "50%",
                        transform: "translateX(-50%)",
                        width: 0,
                        height: 0,
                        borderLeft: "7px solid transparent",
                        borderRight: "7px solid transparent",
                        borderTop: "7px solid rgba(15, 15, 25, 0.92)",
                    }} />
                </motion.div>
            )}
        </AnimatePresence>
    );
}
