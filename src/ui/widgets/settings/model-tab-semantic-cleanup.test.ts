import { describe, expect, it } from "vitest";
import { sanitizeSemanticCueMap } from "./ModelTab";

describe("sanitizeSemanticCueMap", () => {
    it("只保留 interaction: 前缀语义映射", () => {
        const input = {
            "interaction:tap_face": "cue_a",
            "emotion:very_happy": "cue_b",
            "foo:bar": "cue_c",
        };

        expect(sanitizeSemanticCueMap(input)).toEqual({
            "interaction:tap_face": "cue_a",
        });
    });

    it("当全部是 interaction 映射时保持不变", () => {
        const input = {
            "interaction:tap_face": "cue_a",
            "interaction:long_press_head": "cue_b",
        };

        expect(sanitizeSemanticCueMap(input)).toEqual(input);
    });
});
