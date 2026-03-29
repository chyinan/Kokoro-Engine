/**
 * DrawableHitTest — Drawable-level body region detection for Live2D models.
 *
 * Uses Cubism SDK drawable mesh data (vertices + triangle indices) to perform
 * precise hit testing, then maps the hit drawable/part to a semantic body region.
 *
 * This module handles the drawable-name branch of hit testing:
 * 1. Drawable mesh triangle hit test
 * 2. Body region inference from part/drawable names
 *
 * Higher-level orchestration in Live2DViewer decides when to prefer model
 * HitAreas and when to fall back to geometry-based estimation.
 */
import type { Live2DModel } from "pixi-live2d-display/cubism4";
import * as PIXI from "pixi.js";

// ── Body Region Types ─────────────────────────────

export type BodyRegion =
    | "eye" | "mouth" | "face" | "head" | "hair"
    | "hand" | "arm" | "chest" | "body" | "leg"
    | "skirt" | "accessory" | "unknown";

/** Human-readable descriptions for LLM messages */
export const REGION_DESCRIPTIONS: Record<BodyRegion, string> = {
    eye: "eyes",
    mouth: "lips",
    face: "face",
    head: "head",
    hair: "hair",
    hand: "hand",
    arm: "arm",
    chest: "chest",
    body: "body",
    leg: "leg",
    skirt: "skirt",
    accessory: "accessory",
    unknown: "body",
};

// ── Region Mapping Rules ──────────────────────────

interface RegionRule {
    pattern: RegExp;
    region: BodyRegion;
}

/**
 * Ordered rules — first match wins.
 * Supports English, Chinese (简体), and Japanese part/drawable names.
 */
const REGION_RULES: RegionRule[] = [
    { pattern: /eye|瞳|眼睛|EyeBall|目/i, region: "eye" },
    { pattern: /mouth|口|嘴|リップ/i, region: "mouth" },
    { pattern: /face|cheek|nose|ear|brow|脸|鼻|耳|眉|頬|ほほ/i, region: "face" },
    { pattern: /head|头|頭|あたま/i, region: "head" },
    { pattern: /hair|刘海|侧发|后发|前髪|横髪|後ろ髪|髪/i, region: "hair" },
    { pattern: /hand|指|手(?!臂)/i, region: "hand" },
    { pattern: /arm|手臂|腕|うで/i, region: "arm" },
    { pattern: /chest|bust|胸|おっぱい/i, region: "chest" },
    { pattern: /leg|foot|腿|脚|足|あし/i, region: "leg" },
    { pattern: /skirt|裙|スカート/i, region: "skirt" },
    { pattern: /ribbon|hat|帽子|リボン|アクセ/i, region: "accessory" },
    { pattern: /body|neck|身体|首|胴|からだ/i, region: "body" },
];

// ── Core Hit Test ─────────────────────────────────

/**
 * Perform drawable-level hit testing on a Live2D model.
 *
 * @returns The semantic body region name, or null if nothing was hit.
 */
export function drawableHitTest(
    model: Live2DModel,
    globalX: number,
    globalY: number,
    debug = false,
): BodyRegion | null {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const internal = (model as any).internalModel;
    if (!internal?.coreModel) return null;

    const coreModel = internal.coreModel;
    const rawModel = coreModel.getModel();
    if (!rawModel) return null;

    const drawables = rawModel.drawables;
    const parts = rawModel.parts;
    if (!drawables || !parts) return null;

    // Build render-order sorted index list (descending = front-most first)
    const count = drawables.count;

    // Convert PIXI global coordinates → Cubism unit space.
    // toModelPosition returns canvas pixel coords; drawable vertices are in
    // unit space (pixels / PixelsPerUnit, centered at canvas origin).
    const modelPos = model.toModelPosition(new PIXI.Point(globalX, globalY));
    const ci = (coreModel as any)._model?.canvasinfo;
    const ppu: number = ci?.PixelsPerUnit ?? 1;
    const originX: number = ci?.CanvasOriginX ?? 0;
    const originY: number = ci?.CanvasOriginY ?? 0;
    const mx = (modelPos.x - originX) / ppu;
    const my = -(modelPos.y - originY) / ppu;

    if (debug) {
        console.log(`[HitTest] global=(${globalX.toFixed(1)}, ${globalY.toFixed(1)}) → model=(${mx.toFixed(3)}, ${my.toFixed(3)}) drawables=${count}`);
    }

    const sortedIndices = new Array<number>(count);
    for (let i = 0; i < count; i++) sortedIndices[i] = i;
    sortedIndices.sort((a, b) => drawables.renderOrders[b] - drawables.renderOrders[a]);

    // parentPartIndices may exist at runtime even if not in type defs
    const parentPartIndices: Int32Array | undefined = drawables.parentPartIndices;

    for (const i of sortedIndices) {
        // Skip invisible or nearly transparent drawables
        if (drawables.opacities[i] < 0.01) continue;
        if (!coreModel.getDrawableDynamicFlagIsVisible(i)) continue;

        // Skip Cubism built-in HitArea overlay meshes — they are transparent
        // collision regions that would block all real drawable hits.
        const id = drawables.ids[i] ?? "";
        if (/^HitArea/i.test(id)) continue;

        const verts = drawables.vertexPositions[i] as Float32Array;
        const indices = drawables.indices[i] as Uint16Array;
        if (!verts || !indices || verts.length < 4 || indices.length < 3) continue;

        // AABB quick reject
        if (!aabbContains(verts, mx, my)) continue;

        // Triangle-level hit test
        if (!triangleHitTest(verts, indices, mx, my)) continue;

        // Hit! This is the front-most (highest render order) drawable at this point.
        // Stop here — do NOT fall through to deeper layers, which prevents background
        // meshes (e.g. body) from winning over foreground meshes (e.g. hand).
        const drawableId = drawables.ids[i] ?? "";
        let partName = "";

        if (parentPartIndices && parentPartIndices[i] >= 0) {
            const partIdx = parentPartIndices[i];
            partName = parts.ids[partIdx] ?? "";
        }

        const region = resolveBodyRegion(partName, drawableId);

        // Refine "body" hits using Y position — lower portion is likely legs.
        // Threshold is auto-computed from PartBody drawable Y distribution.
        const refined = (region === "body")
            ? refineBodyRegion(my, drawables, parts, debug)
            : region;

        if (debug) {
            console.log(
                `[HitTest] renderOrder=${drawables.renderOrders[i]}` +
                ` | part="${partName}" | drawable="${drawableId}"` +
                ` → region="${region}"${refined !== region ? ` → refined="${refined}"` : ""}`
            );
        }

        // Returns "unknown" if the name doesn't match any region rule.
        // Caller can distinguish "unknown hit" from "no hit at all" (null).
        return refined;
    }

    if (debug) {
        console.log(`[HitTest] No mesh hit at model=(${mx.toFixed(1)}, ${my.toFixed(1)})`);
    }
    return null;
}

// ── Y-Coordinate Fallback ─────────────────────────

/**
 * Rough body region estimation based on vertical position within the model.
 * Used as a last-resort fallback when neither drawable nor HitArea detection works.
 */
export function estimateRegionByY(
    model: Live2DModel,
    globalY: number,
): BodyRegion {
    // Get model bounds in screen space
    const bounds = model.getBounds();
    if (bounds.height <= 0) return "body";

    const ratio = (globalY - bounds.y) / bounds.height;

    if (ratio < 0.15) return "hair";
    if (ratio < 0.30) return "head";
    if (ratio < 0.50) return "chest";
    if (ratio < 0.70) return "body";
    return "leg";
}

// ── Internal Helpers ──────────────────────────────

/** Map a part name + drawable ID to a semantic body region. */
function resolveBodyRegion(partName: string, drawableId: string): BodyRegion {
    // Try part name first (more semantically meaningful)
    const combined = `${partName} ${drawableId}`;
    for (const rule of REGION_RULES) {
        if (rule.pattern.test(combined)) {
            return rule.region;
        }
    }
    return "unknown";
}

/** AABB bounding-box quick reject for a vertex array. */
function aabbContains(verts: Float32Array, px: number, py: number): boolean {
    let minX = Infinity, minY = Infinity;
    let maxX = -Infinity, maxY = -Infinity;

    for (let j = 0; j < verts.length; j += 2) {
        const vx = verts[j];
        const vy = verts[j + 1];
        if (vx < minX) minX = vx;
        if (vx > maxX) maxX = vx;
        if (vy < minY) minY = vy;
        if (vy > maxY) maxY = vy;
    }

    return px >= minX && px <= maxX && py >= minY && py <= maxY;
}

/** Test if point (px, py) is inside any triangle defined by the index buffer. */
function triangleHitTest(
    verts: Float32Array,
    indices: Uint16Array,
    px: number,
    py: number,
): boolean {
    for (let t = 0; t + 2 < indices.length; t += 3) {
        const i0 = indices[t] * 2;
        const i1 = indices[t + 1] * 2;
        const i2 = indices[t + 2] * 2;

        if (pointInTriangle(
            px, py,
            verts[i0], verts[i0 + 1],
            verts[i1], verts[i1 + 1],
            verts[i2], verts[i2 + 1],
        )) {
            return true;
        }
    }
    return false;
}

/** Barycentric coordinate point-in-triangle test. */
function pointInTriangle(
    px: number, py: number,
    ax: number, ay: number,
    bx: number, by: number,
    cx: number, cy: number,
): boolean {
    const v0x = cx - ax, v0y = cy - ay;
    const v1x = bx - ax, v1y = by - ay;
    const v2x = px - ax, v2y = py - ay;

    const dot00 = v0x * v0x + v0y * v0y;
    const dot01 = v0x * v1x + v0y * v1y;
    const dot02 = v0x * v2x + v0y * v2y;
    const dot11 = v1x * v1x + v1y * v1y;
    const dot12 = v1x * v2x + v1y * v2y;

    const denom = dot00 * dot11 - dot01 * dot01;
    if (Math.abs(denom) < 1e-10) return false;

    const invDenom = 1 / denom;
    const u = (dot11 * dot02 - dot01 * dot12) * invDenom;
    const v = (dot00 * dot12 - dot01 * dot02) * invDenom;

    return u >= 0 && v >= 0 && u + v <= 1;
}

/**
 * Refine a "body" region hit using Y position.
 * Scans all PartBody drawables to find the median Y center, then uses
 * the lower 40% of that range as the leg threshold — adapts to each model.
 */
function refineBodyRegion(my: number, drawables: any, parts: any, debug = false): BodyRegion {
    const parentPartIndices: Int32Array | undefined = drawables.parentPartIndices;
    if (!parentPartIndices) return "body";

    const yCenters: number[] = [];
    for (let i = 0; i < drawables.count; i++) {
        if (drawables.opacities[i] < 0.01) continue;
        const partIdx = parentPartIndices[i];
        if (partIdx < 0) continue;
        const partName: string = parts.ids[partIdx] ?? "";
        if (!/body|胴|からだ/i.test(partName)) continue;

        const verts = drawables.vertexPositions[i] as Float32Array;
        if (!verts || verts.length < 4) continue;
        let minY = Infinity, maxY = -Infinity;
        for (let j = 1; j < verts.length; j += 2) {
            if (verts[j] < minY) minY = verts[j];
            if (verts[j] > maxY) maxY = verts[j];
        }
        yCenters.push((minY + maxY) / 2);
    }

    if (debug) {
        console.log(`[HitTest] refineBodyRegion: my=${my.toFixed(3)} yCenters(${yCenters.length})`);
    }

    if (yCenters.length === 0) return "body";

    yCenters.sort((a, b) => a - b);
    const mid = Math.floor(yCenters.length / 2);
    const legThreshold = yCenters.length % 2 === 0
        ? (yCenters[mid - 1] + yCenters[mid]) / 2
        : yCenters[mid];

    if (debug) {
        console.log(`[HitTest] bodyY range=[${yCenters[0].toFixed(3)}, ${yCenters[yCenters.length-1].toFixed(3)}] median(legThreshold)=${legThreshold.toFixed(3)}`);
    }

    return my < legThreshold ? "leg" : "body";
}
