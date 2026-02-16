import { CSSProperties } from "react";

export interface MotionAnimationConfig {
    initial?: Record<string, number | string>;
    animate?: Record<string, number | string>;
    exit?: Record<string, number | string>;
    transition?: Record<string, number | string>;
}

export interface ThemeConfig {
    id: string;
    name: string;
    variables: Record<string, string>;
    assets?: {
        fonts?: string[];
        background?: string;
        noise_texture?: string;
        [key: string]: string | string[] | undefined;
    };
    animations?: Record<string, MotionAnimationConfig>;
}

export interface LayoutNode {
    id: string;
    type: "layer" | "grid" | "split" | "component" | "slot" | "mod-component";
    direction?: "row" | "column"; // For split/grid
    children?: LayoutNode[];
    component?: string; // Registered Component Name
    src?: string; // mod:// URL for mod-component type
    props?: Record<string, unknown>;
    style?: CSSProperties;
    zIndex?: number;
    motion?: string; // Animation preset name
    area?: string; // Grid area name
}

export interface LayoutConfig {
    root: LayoutNode;
}
