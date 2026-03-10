import { useEffect, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";

interface MenuItem {
    label: string;
    onClick: () => void;
    danger?: boolean;
}

interface PetContextMenuProps {
    visible: boolean;
    x: number;
    y: number;
    onClose: () => void;
    items: MenuItem[];
}

export default function PetContextMenu({ visible, x, y, onClose, items }: PetContextMenuProps) {
    const menuRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        if (!visible) return;
        const handleClick = (e: MouseEvent) => {
            if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
                onClose();
            }
        };
        document.addEventListener("mousedown", handleClick);
        return () => document.removeEventListener("mousedown", handleClick);
    }, [visible, onClose]);

    return (
        <AnimatePresence>
            {visible && (
                <motion.div
                    ref={menuRef}
                    initial={{ opacity: 0, scale: 0.92 }}
                    animate={{ opacity: 1, scale: 1 }}
                    exit={{ opacity: 0, scale: 0.92 }}
                    transition={{ duration: 0.12 }}
                    style={{
                        position: "fixed",
                        left: x,
                        top: y,
                        background: "rgba(20, 20, 30, 0.95)",
                        border: "1px solid rgba(255,255,255,0.12)",
                        borderRadius: "10px",
                        padding: "4px",
                        minWidth: "160px",
                        boxShadow: "0 8px 32px rgba(0,0,0,0.5)",
                        backdropFilter: "blur(12px)",
                        zIndex: 9999,
                    }}
                >
                    {items.map((item, i) => (
                        <button
                            key={i}
                            onClick={() => { item.onClick(); onClose(); }}
                            style={{
                                display: "block",
                                width: "100%",
                                padding: "8px 12px",
                                background: "transparent",
                                border: "none",
                                borderRadius: "6px",
                                color: item.danger ? "#ff6b6b" : "#e0e0e0",
                                fontSize: "13px",
                                textAlign: "left",
                                cursor: "pointer",
                                transition: "background 0.1s",
                            }}
                            onMouseEnter={e => (e.currentTarget.style.background = "rgba(255,255,255,0.08)")}
                            onMouseLeave={e => (e.currentTarget.style.background = "transparent")}
                        >
                            {item.label}
                        </button>
                    ))}
                </motion.div>
            )}
        </AnimatePresence>
    );
}
