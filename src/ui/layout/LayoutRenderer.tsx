import { LayoutConfig, LayoutNode } from "./types";
import { registry } from "../registry/ComponentRegistry";
import { IframeSandbox } from "../mods/IframeSandbox";
import { AnimatePresence, motion } from "framer-motion";
import { clsx } from "clsx";
import { useTheme } from "../theme/ThemeContext";

const LayoutNodeRenderer = ({ node }: { node: LayoutNode }) => {
    const { activeTheme } = useTheme();

    // ── mod-component: Render mod iframe directly from layout.json ──
    if (node.type === "mod-component" && node.src) {
        return (
            <div
                className={clsx("relative", node.area)}
                style={{ ...node.style, gridArea: node.area }}
            >
                <IframeSandbox
                    id={node.id}
                    src={node.src}
                    permissions={[]}
                    componentProps={node.props}
                />
            </div>
        );
    }

    if (node.type === "component" && node.component) {
        const Component = registry.get(node.component);

        // Get animation preset from theme or default
        const animation = node.motion && activeTheme?.animations?.[node.motion]
            ? activeTheme.animations[node.motion]
            : undefined; // Only animate if explicitly requested

        const content = Component ? (
            <Component {...node.props} />
        ) : (
            <div className="p-4 text-red-500 bg-red-900/20 border border-red-500/50 rounded">
                Component "{node.component}" not found
            </div>
        );

        // Live2D Stage — full-bleed background layer
        if (node.component === "Live2DStage") {
            return (
                <div
                    style={{ ...node.style, zIndex: node.zIndex || 0 }}
                    className="absolute inset-0 pointer-events-auto"
                >
                    {content}
                </div>
            );
        }

        // Animated UI component
        if (animation) {
            return (
                <motion.div
                    layout
                    {...animation}
                    className={clsx("relative flex flex-col pointer-events-auto overflow-hidden min-h-0", node.area)}
                    style={{ ...node.style, gridArea: node.area }}
                >
                    {content}
                </motion.div>
            );
        }

        // Non-animated UI component
        return (
            <div
                className={clsx("relative flex flex-col overflow-hidden min-h-0", node.area)}
                style={{ ...node.style, gridArea: node.area }}
            >
                {content}
            </div>
        );
    }

    if (node.type === "layer" && node.children) {
        return (
            <div className="relative w-full h-full">
                {node.children.map((child) => (
                    <LayoutNodeRenderer key={child.id} node={child} />
                ))}
            </div>
        );
    }

    if (node.type === "grid" && node.children) {
        return (
            <div
                className="grid w-full h-full"
                style={{
                    ...node.style,
                    zIndex: node.zIndex,
                }}
            >
                {node.children.map((child) => (
                    <LayoutNodeRenderer key={child.id} node={child} />
                ))}
            </div>
        );
    }

    return null;
};

export const LayoutRenderer = ({ config, transparent, backgroundLayer }: { config: LayoutConfig; transparent?: boolean; backgroundLayer?: React.ReactNode }) => {
    return (
        <div className={clsx(
            "w-screen h-screen overflow-hidden text-[var(--color-text-primary)] font-body relative",
            transparent ? "bg-transparent" : "bg-[var(--color-bg-primary)]"
        )}>
            {/* Background image layer — sits behind Live2D */}
            {backgroundLayer}

            {/* Noise texture overlay — §2.3 */}
            <div className="noise-overlay" />

            <AnimatePresence mode="wait">
                <LayoutNodeRenderer node={config.root} />
            </AnimatePresence>
        </div>
    );
};
