import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { OmpProviderForm } from "@/components/providers/forms/OmpProviderForm";

const mocks = vi.hoisted(() => ({
  saveOmpProvider: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    ompApi: {
      ...actual.ompApi,
      saveOmpProvider: mocks.saveOmpProvider,
    },
  };
});

/**
 * OMP 供应商分类落库（2026-09-24 分类对齐）：
 * - 常见供应商预设（tier=common，如腾讯混元）→ category "common"
 * - 内置 API Key 预设（如 DeepSeek）→ category "api"
 * - 自定义入口 → category "custom"（此前从不落盘，一律存成 api）
 * loopback + 无密钥的 local 分支不受影响（见 editSeeding 测试）。
 */
async function selectPreset(name: RegExp) {
  const entry = screen.getByRole("button", { name });
  await userEvent.click(entry);
}

async function save() {
  await userEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(mocks.saveOmpProvider).toHaveBeenCalledTimes(1);
  return mocks.saveOmpProvider.mock.calls[0]![0] as Record<string, unknown>;
}

describe("OmpProviderForm 分类落库", () => {
  it("选择常见供应商预设（腾讯混元）保存为 category common", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    const { container } = render(
      <OmpProviderForm
        submitLabel="保存"
        onSubmit={onSubmit}
        onCancel={() => {}}
      />,
    );

    await selectPreset(/腾讯混元/);

    // 远程 API Key 预设必须填密钥（api-key 校验）
    const apiKeyInput = container.querySelector<HTMLInputElement>(
      'input[type="password"][placeholder^="sk-"]',
    )!;
    await user.type(apiKeyInput, "hy_test_key");

    const saved = await save();
    expect(saved.type).toBe("api-key");
    expect(saved.category).toBe("common");
    expect(saved.baseUrl).toContain("hunyuan");
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("选择内置 API Key 预设（DeepSeek）保存为 category api", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    const { container } = render(
      <OmpProviderForm
        submitLabel="保存"
        onSubmit={onSubmit}
        onCancel={() => {}}
      />,
    );

    await selectPreset(/DeepSeek API/);

    const apiKeyInput = container.querySelector<HTMLInputElement>(
      'input[type="password"][placeholder^="sk-"]',
    )!;
    await user.type(apiKeyInput, "sk_test_key");

    const saved = await save();
    expect(saved.type).toBe("api-key");
    expect(saved.category).toBe("api");
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("自定义入口保存为 category custom（不再静默归为 api）", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    const { container } = render(
      <OmpProviderForm
        submitLabel="保存"
        onSubmit={onSubmit}
        onCancel={() => {}}
      />,
    );

    // 新建态默认选中「自定义」入口
    const nameInput = container.querySelector<HTMLInputElement>(
      'input[name="name"]',
    )!;
    await user.type(nameInput, "My Relay");

    const baseUrlInput =
      container.querySelector<HTMLInputElement>('input[type="url"]')!;
    fireEvent.change(baseUrlInput, {
      target: { value: "https://relay.example.com/v1" },
    });

    const apiKeyInput = container.querySelector<HTMLInputElement>(
      'input[type="password"][placeholder^="sk-"]',
    )!;
    await user.type(apiKeyInput, "sk_relay_key");

    const saved = await save();
    expect(saved.type).toBe("api-key");
    expect(saved.category).toBe("custom");
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("编辑既有 common 条目时 category 保真（不漂移回 api）", async () => {
    const onSubmit = vi.fn();
    render(
      <OmpProviderForm
        submitLabel="保存"
        onSubmit={onSubmit}
        onCancel={() => {}}
        providerId="hunyuan"
        initialData={{
          name: "腾讯混元",
          category: "third_party",
          settingsConfig: {
            config: {
              id: "hunyuan",
              name: "腾讯混元",
              type: "api-key",
              category: "common",
              models: [],
              baseUrl: "https://api.hunyuan.cloud.tencent.com/v1",
              apiKey: "hy_key",
              api: "openai-completions",
            },
          },
        }}
      />,
    );

    const saved = await save();
    expect(saved.category).toBe("common");
    expect(saved.apiKey).toBe("hy_key");
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });
});
