/**
 * DrawableHitTest — Drawable-level body region detection for Live2D models.
 *
 * Uses Cubism SDK drawable mesh data (vertices + triangle indices) to perform
 * precise hit testing, then maps the hit drawable/part to a semantic body region.
 *
 * Three-level fallback:
 * 1. Drawable mesh triangle hit test → body region from part/drawable name
 * 2. Original HitArea detection (for models that define them)
 * 3. Y-coordinate estimation (rough vertical split)
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

    // Convert PIXI global coordinates → model canvas space
    const modelPos = model.toModelPosition(new PIXI.Point(globalX, globalY));
    const mx = modelPos.x;
    const my = modelPos.y;

    // Build render-order sorted index list (descending = front-most first)
    const count = drawables.count;
    const sortedIndices = new Array<number>(count);
    for (let i = 0; i < count; i++) sortedIndices[i] = i;
    sortedIndices.sort((a, b) => drawables.renderOrders[b] - drawables.renderOrders[a]);

    // parentPartIndices may exist at runtime even if not in type defs
    const parentPartIndices: Int32Array | undefined = drawables.parentPartIndices;

    for (const i of sortedIndices) {
        // Skip invisible or nearly transparent drawables
        if (drawables.opacities[i] < 0.01) continue;
        if (!coreModel.getDrawableDynamicFlagIsVisible(i)) continue;

        const verts = drawables.vertexPositions[i] as Float32Array;
        const indices = drawables.indices[i] as Uint16Array;
        if (!verts || !indices || verts.length < 4 || indices.length < 3) continue;

        // AABB quick reject
        if (!aabbContains(verts, mx, my)) continue;

        // Triangle-level hit test
        if (!triangleHitTest(verts, indices, mx, my)) continue;

        // Hit! Resolve body region from part name or drawable ID
        const drawableId = drawables.ids[i] ?? "";
        let partName = "";

        if (parentPartIndices && parentPartIndices[i] >= 0) {
            const partIdx = parentPartIndices[i];
            partName = parts.ids[partIdx] ?? "";
        }

        const region = resolveBodyRegion(partName, drawableId);
        if (region !== "unknown") {
            return region;
        }

        // If this drawable didn't map to a known region, keep searching
        // (it might be a mask or decoration mesh)
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
