import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { OmpModelListEditor } from "@/components/providers/forms/OmpModelListEditor";
import type { OmpModelInfo } from "@/types/omp";

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));

// 捕获 DndContext 的 onDragEnd，直接以 (activeId, overId) 触发重排，
// 与 tests/hooks/useDragSort.test.tsx 的模拟方式一致。
let dragEndHandler: ((active: string, over: string) => void) | null = null;
vi.mock("@dnd-kit/core", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("@dnd-kit/core")>();
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
  const actual =
    await importOriginal<typeof import("@dnd-kit/sortable")>();
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

const model = (id: string): OmpModelInfo => ({
  id,
  name: id,
  contextWindow: 0,
  maxTokens: 0,
});

// 行 id 由 crypto.randomUUID() 生成；stub 成确定性 id 以便在 onDragEnd 里引用
let uuidCounter = 0;
vi.stubGlobal("crypto", {
  ...globalThis.crypto,
  randomUUID: () => `row-${++uuidCounter}`,
});

describe("OmpModelListEditor 拖动排序", () => {
  beforeEach(() => {
    dragEndHandler = null;
    uuidCounter = 0;
  });

  it("每一行都有拖拽手柄", () => {
    render(
      <OmpModelListEditor
        models={[model("a"), model("b"), model("c")]}
        onModelsChange={() => {}}
      />,
    );
    const handles = screen.getAllByRole("button", { name: /拖拽排序/ });
    expect(handles).toHaveLength(3);
  });

  it("拖动后按新顺序回调（顺序即写进 models.yml 的顺序）", async () => {
    const onModelsChange = vi.fn();
    render(
      <OmpModelListEditor
        models={[model("a"), model("b"), model("c")]}
        onModelsChange={onModelsChange}
      />,
    );

    expect(dragEndHandler).not.toBeNull();
    (dragEndHandler as unknown as (a: string, o: string) => void)("row-3", "row-1");

    expect(onModelsChange).toHaveBeenCalledTimes(1);
    const next = onModelsChange.mock.calls[0]![0] as OmpModelInfo[];
    expect(next.map((m) => m.id)).toEqual(["c", "a", "b"]);
  });

  it("拖到原位不触发回调", () => {
    const onModelsChange = vi.fn();
    render(
      <OmpModelListEditor models={[model("a"), model("b")]} onModelsChange={onModelsChange} />,
    );
    (dragEndHandler as unknown as (a: string, o: string) => void)("row-1", "row-1");
    expect(onModelsChange).not.toHaveBeenCalled();
  });

  it("行内输入框仍可正常编辑", async () => {
    const user = userEvent.setup();
    const onModelsChange = vi.fn();
    render(
      <OmpModelListEditor
        models={[model("a")]}
        onModelsChange={onModelsChange}
      />,
    );
    const input = screen.getByPlaceholderText("model-id");
    await user.type(input, "x");
    expect(onModelsChange).toHaveBeenCalled();
    const last = onModelsChange.mock.calls.at(-1)![0] as OmpModelInfo[];
    expect(last[0]!.id).toBe("ax");
  });
});
