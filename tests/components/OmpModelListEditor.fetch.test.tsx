import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { OmpModelListEditor } from "@/components/providers/forms/OmpModelListEditor";
import type { OmpModelInfo } from "@/types/omp";

const mocks = vi.hoisted(() => ({
  ompListModels: vi.fn(),
  ompFetchUpstreamModels: vi.fn(),
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));

vi.mock("sonner", () => ({ toast: mocks.toast }));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    ompApi: {
      ...actual.ompApi,
      ompListModels: mocks.ompListModels,
      ompFetchUpstreamModels: mocks.ompFetchUpstreamModels,
    },
  };
});

/** 最近一次成功 toast 的文案（组件用它区分「上游目录」与「OMP 目录」两个来源） */
const lastSuccessMessage = (): string =>
  (mocks.toast.success.mock.calls.at(-1)?.[0] as string) ?? "";

const model = (id: string): OmpModelInfo => ({
  id,
  name: id,
  contextWindow: 0,
  maxTokens: 0,
});

const configured: OmpModelInfo[] = [model("cn:old")];

describe("OmpModelListEditor 获取模型列表", () => {
  it("有凭据时以上游目录为准，不被已配置的模型清单锁死", async () => {
    const user = userEvent.setup();
    const onModelsChange = vi.fn();
    // OMP 目录对自定义供应商只返回 models.yml 里已配置的条目（历史 bug 的根源）
    mocks.ompListModels.mockResolvedValue([
      { ...model("cn:hy3"), contextWindow: 188000, maxTokens: 63000, reasoning: true },
    ]);
    mocks.ompFetchUpstreamModels.mockResolvedValue([
      model("cn:hy3"),
      model("cn:deepseek-v4.1-flash"),
      model("cn:kimi-k2.8-preview"),
    ]);

    render(
      <OmpModelListEditor
        models={configured}
        onModelsChange={onModelsChange}
        baseUrl="http://127.0.0.1:7864/v1"
        apiKey="wbk_test"
        authHeader
        providerId="workbuddy"
      />,
    );

    await user.click(screen.getByRole("button", { name: /获取模型列表/ }));

    await screen.findByText(/已获取 3 个模型/);
    await waitFor(() => expect(mocks.toast.success).toHaveBeenCalled());
    expect(lastSuccessMessage()).toContain("获取到 3 个模型");
    expect(lastSuccessMessage()).not.toContain("OMP 模型目录");
    expect(mocks.ompFetchUpstreamModels).toHaveBeenCalledTimes(1);

    // 上游全量可导入（不再只有已配置的 cn:old）
    await user.click(screen.getByRole("button", { name: /全部导入/ }));
    expect(onModelsChange).toHaveBeenCalledTimes(1);
    const next = onModelsChange.mock.calls[0]![0] as OmpModelInfo[];
    expect(next.map((m) => m.id)).toEqual([
      "cn:old",
      "cn:hy3",
      "cn:deepseek-v4.1-flash",
      "cn:kimi-k2.8-preview",
    ]);
    // 上游不返回元数据，同 id 的条目用 OMP 目录补齐
    expect(next[1]!.contextWindow).toBe(188000);
    expect(next[1]!.maxTokens).toBe(63000);
    expect(next[1]!.reasoning).toBe(true);
  });

  it("没有凭据（OAuth 供应商）时走 OMP 原生目录", async () => {
    const user = userEvent.setup();
    mocks.ompListModels.mockResolvedValue([model("gpt-5.6-terra")]);

    render(
      <OmpModelListEditor
        models={[]}
        onModelsChange={() => {}}
        providerId="openai-codex"
      />,
    );

    await user.click(screen.getByRole("button", { name: /获取模型列表/ }));

    await screen.findByText(/已获取 1 个模型/);
    await waitFor(() => expect(mocks.toast.success).toHaveBeenCalled());
    expect(lastSuccessMessage()).toContain("从 OMP 模型目录获取到 1 个模型");
    expect(mocks.ompListModels).toHaveBeenCalledWith("openai-codex");
    expect(mocks.ompFetchUpstreamModels).not.toHaveBeenCalled();
  });

  it("上游不可用时回落到 OMP 原生目录", async () => {
    const user = userEvent.setup();
    mocks.ompListModels.mockResolvedValue([model("cn:hy3")]);
    mocks.ompFetchUpstreamModels.mockRejectedValue(new Error("上游返回 401"));

    render(
      <OmpModelListEditor
        models={configured}
        onModelsChange={() => {}}
        baseUrl="https://relay.example.com/v1"
        apiKey="sk-bad"
        providerId="workbuddy"
      />,
    );

    await user.click(screen.getByRole("button", { name: /获取模型列表/ }));

    await screen.findByText(/已获取 1 个模型/);
    await waitFor(() => expect(mocks.toast.success).toHaveBeenCalled());
    expect(lastSuccessMessage()).toContain("从 OMP 模型目录获取到 1 个模型");
    expect(mocks.ompFetchUpstreamModels).toHaveBeenCalledTimes(1);
  });
});
