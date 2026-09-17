import { describe, expect, it } from "vitest";
import {
  defaultKeyAfterRemoval,
  reconcileDefaultKey,
  type GrokModelEntry,
  updateGrokModelEntries,
  validateGrokModelEntries,
} from "@/utils/grokBuildConfig";

const entry = (id: string, displayName = id): GrokModelEntry => ({
  id,
  displayName,
  contextWindow: 500000,
  extra: {},
});

const A = entry("grok-4.5");
const B = entry("grok-4.6");
const C = entry("grok-4.3");

describe("reconcileDefaultKey", () => {
  it("保留仍然存在于列表里的默认值", () => {
    expect(reconcileDefaultKey([A, B], "grok-4.6")).toBe("grok-4.6");
  });

  it("默认值指向不存在的条目时回落到首个条目", () => {
    // 这是"默认行的模型 ID 被改名"后的状态：默认值悬空。
    expect(reconcileDefaultKey([B, C], "grok-4.5")).toBe("grok-4.6");
  });

  it("忽略首尾空白后再匹配", () => {
    expect(reconcileDefaultKey([A, B], "  grok-4.5  ")).toBe("grok-4.5");
  });

  it("空列表返回空串", () => {
    expect(reconcileDefaultKey([], "grok-4.5")).toBe("");
  });

  it("默认值为空时取首个条目", () => {
    expect(reconcileDefaultKey([A, B], "")).toBe("grok-4.5");
  });
});

describe("defaultKeyAfterRemoval", () => {
  it("删除非默认行时保留原默认值", () => {
    // 列表 [A, B, C]，默认 A；删掉中间的 B。
    expect(defaultKeyAfterRemoval([A, B, C], 1, "grok-4.5")).toBe("grok-4.5");
  });

  it("删除默认行时顺延到原位置上的下一行", () => {
    // 默认是中间的 B，删掉 B → 顺延到原索引 1 上的 C。
    expect(defaultKeyAfterRemoval([A, B, C], 1, "grok-4.6")).toBe("grok-4.3");
  });

  it("删除末尾的默认行时取上一行", () => {
    expect(defaultKeyAfterRemoval([A, B, C], 2, "grok-4.3")).toBe("grok-4.6");
  });

  it("删除首个默认行时取新的首个", () => {
    expect(defaultKeyAfterRemoval([A, B, C], 0, "grok-4.5")).toBe("grok-4.6");
  });

  it("默认值已悬空时，删行后回落到存活的首个条目", () => {
    // 默认值 grok-4.9 不存在；删掉索引 0 → 结果应为存活的首个。
    expect(defaultKeyAfterRemoval([A, B], 0, "grok-4.9")).toBe("grok-4.6");
  });

  it("删到只剩一行时该行成为默认", () => {
    expect(defaultKeyAfterRemoval([A], 0, "grok-4.5")).toBe("");
    expect(defaultKeyAfterRemoval([A, B], 0, "grok-4.6")).toBe("grok-4.6");
  });
});

describe("删除默认行的完整往返（写入 + 校验）", () => {
  it("顺延后的默认值能通过校验，且配置里已无被删条目", () => {
    const entries = [A, B, C];
    const removedIndex = 1; // 删掉默认的 B
    const nextDefaultKey = defaultKeyAfterRemoval(entries, removedIndex, "grok-4.6");
    const nextEntries = entries.filter((_, i) => i !== removedIndex);

    // 校验必须通过——这正是此前会持续报"默认模型 X 不在条目列表里"的地方。
    expect(
      validateGrokModelEntries(
        { entries: nextEntries, defaultKey: nextDefaultKey, modelsExtra: {} },
        { baseUrl: "https://example.com/v1", apiKey: "sk-test" },
      ),
    ).toBeNull();

    const toml = updateGrokModelEntries(
      "",
      { entries: nextEntries, defaultKey: nextDefaultKey, modelsExtra: {} },
      { baseUrl: "https://example.com/v1", apiKey: "sk-test" },
    );
    expect(toml).toContain('default = "grok-4.3"');
    expect(toml).not.toContain('"grok-4.6"'); // 被删的行不得残留在配置里
  });
});
