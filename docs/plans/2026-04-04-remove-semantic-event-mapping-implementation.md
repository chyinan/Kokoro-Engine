# 移除 Semantic Event Mapping Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 删除前端设置页中的 Semantic Event Mapping 区块，并在读取模型配置时自动清理所有非 `interaction:` 语义映射。

**Architecture:** 仅改前端 `ModelTab` 与 i18n 资源文件。UI 层移除 semantic 区块和对应状态/方法；数据层在 profile 读取后做一次 `semantic_cue_map` 规范化并持久化（仅保留 `interaction:*`）。同时在 `en/ja/ko/zh` 四套语言中删除已废弃 semantic key，保持 interaction 映射能力不变。

**Tech Stack:** React 19 + TypeScript + Vitest + i18next（`src/ui/locales/{en,ja,ko,zh}.json`）

---

### Task 1: 新增最小回归测试（RED）

**Files:**
- Create: `src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts`
- Modify: `src/ui/widgets/settings/ModelTab.tsx`（仅在后续任务实现时）
- Test: `src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts`

**Step 1: 写失败测试（只测一个行为）**

在 `model-tab-semantic-cleanup.test.ts` 中先提取/新增可测试的纯函数（若不存在先按“期望接口”写测试）：
- `sanitizeSemanticCueMap(input: Record<string, string>): Record<string, string>`

失败测试示例：
```ts
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
});
```

**Step 2: 运行测试确认失败**

Run:
```bash
npm run test -- src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
```

Expected: FAIL（`sanitizeSemanticCueMap` 未导出或行为不匹配）

**Step 3: 提交测试（可选小提交）**

```bash
git add src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
git commit -m "test: add semantic map cleanup regression case"
```

---

### Task 2: 实现最小代码使测试通过（GREEN）

**Files:**
- Modify: `src/ui/widgets/settings/ModelTab.tsx`
- Test: `src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts`

**Step 1: 实现最小纯函数**

在 `ModelTab.tsx` 增加并导出：
```ts
export function sanitizeSemanticCueMap(input: Record<string, string>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(input).filter(([key]) => key.startsWith("interaction:")),
  );
}
```

**Step 2: 运行单测确认通过**

Run:
```bash
npm run test -- src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
```

Expected: PASS

**Step 3: 提交**

```bash
git add src/ui/widgets/settings/ModelTab.tsx src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
git commit -m "feat: keep only interaction semantic mappings"
```

---

### Task 3: 删除 Semantic Event Mapping UI 与相关状态/方法

**Files:**
- Modify: `src/ui/widgets/settings/ModelTab.tsx`

**Step 1: 写失败测试（UI 不再出现语义映射标题）**

在 `model-tab-semantic-cleanup.test.ts` 增加第二个测试（可用最小渲染+文本断言；如当前项目没有 RTL，可改为结构级单测 + 后续人工验证）。

建议测试目标：
- 不再引用 `settings.model.mapping.semantic.title`
- Interaction 区块 key `settings.model.mapping.interactions.title` 仍存在引用

（如果当前测试基础设施不支持组件渲染，保持“字符串/导出常量级”断言，避免引入新测试框架依赖）

**Step 2: 运行测试确认失败**

Run:
```bash
npm run test -- src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
```

Expected: FAIL（因为语义区块仍在）

**Step 3: 最小实现删除 UI 和逻辑**

从 `ModelTab.tsx` 删除：
- `SEMANTIC_KEYS`
- `semanticKeyLabel`
- `semanticKey` / `semanticCue` / `editingSemanticKey`
- `resetSemanticDraft`
- `handleSaveSemanticMapping` / `handleEditSemanticMapping` / `handleDeleteSemanticMapping`
- `generalSemanticEntries`
- JSX 中整个 `settings.model.mapping.semantic.*` 区块

保留：
- interaction 区块与其编辑行为
- `persistSemanticMap`（interaction 仍依赖）

**Step 4: 运行测试确认通过**

Run:
```bash
npm run test -- src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
```

Expected: PASS

**Step 5: 提交**

```bash
git add src/ui/widgets/settings/ModelTab.tsx src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
git commit -m "refactor: remove semantic event mapping ui from model settings"
```

---

### Task 4: 在 profile 加载后自动清理非 interaction 语义映射

**Files:**
- Modify: `src/ui/widgets/settings/ModelTab.tsx`
- Test: `src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts`

**Step 1: 写失败测试（规范化触发条件）**

新增单测验证：
- 输入 map 含非 interaction 键时，`sanitizeSemanticCueMap` 与原 map 不等
- 输入仅 interaction 键时，输出等于输入

**Step 2: 运行测试确认失败**

Run:
```bash
npm run test -- src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
```

Expected: FAIL（覆盖不足或实现未接入）

**Step 3: 实现最小接入逻辑**

在 `getLive2dModelProfile(effectiveModelPath)` 成功后：
1. 计算 `sanitized = sanitizeSemanticCueMap(profile.semantic_cue_map ?? {})`
2. 若 `Object.keys(sanitized).length !== Object.keys(profile.semantic_cue_map ?? {}).length`：
   - 调用 `saveLive2dModelProfile({ ...profile, semantic_cue_map: sanitized })`
   - `setModelProfile(saved)`
3. 否则 `setModelProfile(profile)`

确保异常路径仍能落到现有 `catch`。

**Step 4: 运行测试确认通过**

Run:
```bash
npm run test -- src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
```

Expected: PASS

**Step 5: 提交**

```bash
git add src/ui/widgets/settings/ModelTab.tsx src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
git commit -m "fix: auto-clean non-interaction semantic mappings on profile load"
```

---

### Task 5: 清理多语言文案 key（i18n）

**Files:**
- Modify: `src/ui/locales/en.json`
- Modify: `src/ui/locales/ja.json`
- Modify: `src/ui/locales/ko.json`
- Modify: `src/ui/locales/zh.json`
- Verify refs: `src/ui/widgets/settings/ModelTab.tsx`

**Step 1: 写失败校验（引用清理）**

增加/复用测试检查以下 key 不再被 `ModelTab` 引用：
- `settings.model.mapping.semantic.title`
- `settings.model.mapping.semantic.desc`
- `settings.model.mapping.semantic.configured`
- `settings.model.mapping.semantic.empty`
- `settings.model.mapping.actions.save_semantic`

**Step 2: 运行测试确认失败**

Run:
```bash
npm run test -- src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
```

Expected: FAIL（旧引用存在）

**Step 3: 最小实现文案删除（四语言一致）**

从以下文件同步删除 semantic 专用项（仅删除不再引用的键）：
- `src/ui/locales/en.json`
- `src/ui/locales/ja.json`
- `src/ui/locales/ko.json`
- `src/ui/locales/zh.json`

删除目标：
- `settings.model.mapping.semantic.*`
- `settings.model.mapping.actions.save_semantic`
- `settings.model.mapping.semantic_keys.*`（确认无引用后删除）

要求：四语言 key 结构保持一致，避免后续 i18n 漏洞或 fallback 异常。

**Step 4: 运行测试确认通过**

Run:
```bash
npm run test -- src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
```

Expected: PASS

**Step 5: 提交**

```bash
git add src/ui/locales/en.json src/ui/locales/ja.json src/ui/locales/ko.json src/ui/locales/zh.json src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
git commit -m "chore: remove obsolete semantic mapping locale keys across locales"
```

---

### Task 6: 最小全量验证（完成前必做）

**Files:**
- Verify: `src/ui/widgets/settings/ModelTab.tsx`
- Verify: `src/ui/locales/en.json`
- Verify: `src/ui/locales/ja.json`
- Verify: `src/ui/locales/ko.json`
- Verify: `src/ui/locales/zh.json`
- Verify: `src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts`

**Step 1: 运行目标测试**

Run:
```bash
npm run test -- src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
```

Expected: PASS

**Step 2: 运行前端类型检查**

Run:
```bash
npx tsc --noEmit
```

Expected: 无错误

**Step 3: （可选）运行现有 settings 相关测试**

Run:
```bash
npm run test -- src/ui/widgets/settings/mcpToolDisplay.test.ts
```

Expected: PASS

**Step 4: 最终提交（若前面未分提交）**

```bash
git add src/ui/widgets/settings/ModelTab.tsx src/ui/locales/en.json src/ui/locales/ja.json src/ui/locales/ko.json src/ui/locales/zh.json src/ui/widgets/settings/model-tab-semantic-cleanup.test.ts
git commit -m "refactor: remove semantic event mapping and clean stale semantic entries"
```

---

## 实施备注
- 遵循 YAGNI：不新增后端接口，不引入新测试库。
- FCIS：`sanitizeSemanticCueMap` 保持纯函数，便于单测。
- TDD：每个行为先写失败测试，再最小实现。
- 若测试基础设施不足以渲染组件，优先保证“纯函数 + 引用清理”可验证，再配合一次人工 UI 验收。

## 完成定义（DoD）
- Semantic Event Mapping UI 不再出现。
- 加载旧 profile 时，非 `interaction:` 语义映射自动清理并写回。
- interaction 映射行为保持可用。
- `zh.json` 不再包含已废弃 semantic 专用键。
- 指定测试与类型检查通过。