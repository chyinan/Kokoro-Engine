import { describe, expect, it } from "vitest";
import {
    restoreStructuredMemoryPrefix,
    splitStructuredMemoryContent,
    stripStructuredMemoryPrefix,
} from "./memory-display-content";

describe("memory display content", () => {
    it("strips structured metadata prefix from memory content", () => {
        expect(
            stripStructuredMemoryPrefix("[type:profile|key:user] 浅川枫的本名是浅川枫"),
        ).toBe("浅川枫的本名是浅川枫");
    });

    it("keeps normal bracketed user content intact", () => {
        expect(
            stripStructuredMemoryPrefix("[time mcp] 这个名字是用户自己提到的"),
        ).toBe("[time mcp] 这个名字是用户自己提到的");
    });

    it("restores structured metadata prefix when saving edited content", () => {
        const parsed = splitStructuredMemoryContent("[type:profile|key:user] 原始内容");

        expect(restoreStructuredMemoryPrefix(parsed.prefix, "修改后的内容")).toBe(
            "[type:profile|key:user] 修改后的内容",
        );
    });
});
