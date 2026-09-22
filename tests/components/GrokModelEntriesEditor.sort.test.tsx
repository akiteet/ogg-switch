import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { GrokModelEntriesEditor } from "@/components/providers/forms/GrokModelEntriesEditor";
import type { GrokModelEntry } from "@/utils/grokBuildConfig";

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));

// 捕获 DndContext 的 onDragEnd，直接以 (activeId, overId) 触发重排
let dragEndHandler: ((active: string, over: string) => void) | null = null;
vi.mock("@dnd-kit/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@dnd-kit/core")>();
  return {
    ...actual,
    DndContext: ({
      children,
      onDragEnd,
    }: {
      children: React.ReactNode;
      onDragEnd: (event: { active: { id: unknown }; over: { id: unknown } | null }) => void;
    }) => {
      dragEndHandler = (active: string, over: string) =>
        onDragEnd({ active: { id: active }, over: { id: over } });
      return <>{children}</>;
    },
  };
});

vi.mock("@dnd-kit/sortable", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@dnd-kit/sortable")>();
  return {
    ...actual,
    useSortable: ({ id }: { id: string }) => ({
      setNodeRef: undefined,
      attributes: { "data-row-id": id },
      listeners: {},
      transform: null,
      transition: undefined,
      isDragging: false,
    }),
  };
});

const entry = (id: string, contextWindow = 500000): GrokModelEntry => ({
  id,
  displayName: id,
  contextWindow,
  extra: {},
});

// 行 id 由 crypto.randomUUID() 生成；stub 成确定性 id 以便在 onDragEnd 里引用
let uuidCounter = 0;
vi.stubGlobal("crypto", {
  ...globalThis.crypto,
  randomUUID: () => `row-${++uuidCounter}`,
});

describe("GrokModelEntriesEditor 拖动排序", () => {
  beforeEach(() => {
    dragEndHandler = null;
    uuidCounter = 0;
  });

  it("每一行都有拖拽手柄", () => {
    render(
      <GrokModelEntriesEditor
        entries={[entry("grok-4.6"), entry("grok-4.5")]}
        defaultKey="grok-4.6"
        onChange={() => {}}
        baseUrl="https://api.x.ai/v1"
        apiKey="sk-test"
      />,
    );
    expect(screen.getAllByRole("button", { name: /拖拽排序/ })).toHaveLength(2);
  });

  it("拖动后按新顺序回调，默认模型跟着行走", () => {
    const onChange = vi.fn();
    render(
      <GrokModelEntriesEditor
        entries={[entry("grok-4.6"), entry("grok-4.5")]}
        // 默认行是 grok-4.6（第一行）；把它拖到第二行后默认值应仍是 grok-4.6
        defaultKey="grok-4.6"
        onChange={onChange}
        baseUrl=""
        apiKey=""
      />,
    );

    (dragEndHandler as unknown as (a: string, o: string) => void)("row-1", "row-2");

    expect(onChange).toHaveBeenCalledTimes(1);
    const { entries: next, defaultKey } = onChange.mock.calls[0]![0] as {
      entries: GrokModelEntry[];
      defaultKey: string;
    };
    expect(next.map((e) => e.id)).toEqual(["grok-4.5", "grok-4.6"]);
    expect(defaultKey).toBe("grok-4.6");
  });

  it("拖动非默认行不改变默认模型", () => {
    const onChange = vi.fn();
    render(
      <GrokModelEntriesEditor
        entries={[entry("grok-4.6"), entry("grok-4.5"), entry("grok-3")]}
        defaultKey="grok-4.6"
        onChange={onChange}
        baseUrl=""
        apiKey=""
      />,
    );

    (dragEndHandler as unknown as (a: string, o: string) => void)("row-3", "row-1");

    const { entries: next, defaultKey } = onChange.mock.calls[0]![0] as {
      entries: GrokModelEntry[];
      defaultKey: string;
    };
    expect(next.map((e) => e.id)).toEqual(["grok-3", "grok-4.6", "grok-4.5"]);
    expect(defaultKey).toBe("grok-4.6");
  });
});
