import { describe, expect, it } from "vitest";
import { reorderAligned, syncRowIds } from "@/components/providers/forms/shared/rowReorder";

describe("reorderAligned", () => {
  it("按行 id 移动内容元素（后往前移）", () => {
    const rows = ["r1", "r2", "r3"];
    const items = ["a", "b", "c"];
    expect(reorderAligned(rows, items, "r3", "r1")).toEqual(["c", "a", "b"]);
  });

  it("按行 id 移动内容元素（前往后移）", () => {
    const rows = ["r1", "r2", "r3"];
    const items = ["a", "b", "c"];
    expect(reorderAligned(rows, items, "r1", "r3")).toEqual(["b", "c", "a"]);
  });

  it("active 与 over 相同时原样返回", () => {
    const items = ["a", "b"];
    expect(reorderAligned(["r1", "r2"], items, "r1", "r1")).toBe(items);
  });

  it("行 id 不存在时不改动", () => {
    const items = ["a", "b"];
    expect(reorderAligned(["r1", "r2"], items, "rx", "r1")).toBe(items);
    expect(reorderAligned(["r1", "r2"], items, "r1", "rx")).toBe(items);
  });

  it("内容可以是任意对象", () => {
    const items = [
      { id: "a", v: 1 },
      { id: "b", v: 2 },
    ];
    expect(reorderAligned(["r1", "r2"], items, "r2", "r1")).toEqual([
      { id: "b", v: 2 },
      { id: "a", v: 1 },
    ]);
  });
});

describe("syncRowIds", () => {
  it("数量一致时保持原列表", () => {
    expect(syncRowIds(["r1", "r2"], 2, () => "new")).toEqual(["r1", "r2"]);
  });

  it("不足时补新 id（新增行）", () => {
    expect(syncRowIds(["r1"], 3, () => "new")).toEqual(["r1", "new", "new"]);
  });

  it("超出时截断（删除行）", () => {
    expect(syncRowIds(["r1", "r2", "r3"], 2, () => "new")).toEqual(["r1", "r2"]);
  });

  it("空列表从零生成", () => {
    let n = 0;
    expect(syncRowIds([], 2, () => `id-${++n}`)).toEqual(["id-1", "id-2"]);
  });
});
