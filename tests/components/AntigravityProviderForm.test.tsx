import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AntigravityProviderForm } from "@/components/providers/forms/AntigravityProviderForm";

const mocks = vi.hoisted(() => ({
  antigravityListModels: vi.fn(),
}));

// radix 下拉打开时会调用 scrollIntoView（jsdom 未实现）
Element.prototype.scrollIntoView = vi.fn();

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    providersApi: {
      ...actual.providersApi,
      antigravityListModels: mocks.antigravityListModels,
    },
  };
});

const baseProps = {
  submitLabel: "保存",
  onSubmit: vi.fn().mockResolvedValue(undefined),
  onCancel: () => {},
};

describe("AntigravityProviderForm 默认模型（settings.json:model）", () => {
  it("编辑态从顶层 model 字段回显（不再读 env.GEMINI_MODEL）", () => {
    const { container } = render(
      <AntigravityProviderForm
        {...baseProps}
        initialData={{
          name: "Relay",
          settingsConfig: {
            authType: "api-key",
            // legacy 残留：env 里的 GEMINI_MODEL 应被无视
            env: {
              GEMINI_API_KEY: "sk-test",
              GEMINI_MODEL: "gemini-3.1-pro-preview",
            },
            model: "Gemini 3.8 Flash (Low)",
          },
        }}
      />,
    );

    const input = container.querySelector<HTMLInputElement>(
      "#antigravity-default-model",
    )!;
    expect(input).not.toBeNull();
    expect(input.value).toBe("Gemini 3.8 Flash (Low)");
  });

  it("保存时写顶层 model 键，env 不含 GEMINI_MODEL", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { container } = render(
      <AntigravityProviderForm
        submitLabel="保存"
        onSubmit={onSubmit}
        onCancel={() => {}}
        initialData={{
          name: "Relay",
          settingsConfig: {
            authType: "api-key",
            env: { GEMINI_API_KEY: "sk-old", GEMINI_MODEL: "legacy-value" },
            model: "Old Model",
          },
        }}
      />,
    );

    // 改默认模型
    const modelInput = container.querySelector<HTMLInputElement>(
      "#antigravity-default-model",
    )!;
    fireEvent.change(modelInput, { target: { value: "Gemini 3.8 Flash (Low)" } });

    await user.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    const payload = onSubmit.mock.calls[0]![0] as { settingsConfig: string };
    const config = JSON.parse(payload.settingsConfig) as Record<string, unknown>;
    expect(config.model).toBe("Gemini 3.8 Flash (Low)");
    const env = config.env as Record<string, string>;
    expect(env.GEMINI_MODEL).toBeUndefined();
    expect(env.GEMINI_API_KEY).toBe("sk-old");
  });

  it("默认模型留空时 settingsConfig 不带 model 键（后端保留 agy 里的选择）", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { container } = render(
      <AntigravityProviderForm
        submitLabel="保存"
        onSubmit={onSubmit}
        onCancel={() => {}}
        initialData={{
          name: "Relay",
          settingsConfig: {
            authType: "api-key",
            env: { GEMINI_API_KEY: "sk-old" },
            model: "Will Be Cleared",
          },
        }}
      />,
    );

    const modelInput = container.querySelector<HTMLInputElement>(
      "#antigravity-default-model",
    )!;
    fireEvent.change(modelInput, { target: { value: "" } });

    await user.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    const payload = onSubmit.mock.calls[0]![0] as { settingsConfig: string };
    const config = JSON.parse(payload.settingsConfig) as Record<string, unknown>;
    expect("model" in config).toBe(false);
  });

  it("legacy env.GEMINI_MODEL 残留不显示为额外环境变量", () => {
    const { container } = render(
      <AntigravityProviderForm
        {...baseProps}
        initialData={{
          name: "Relay",
          settingsConfig: {
            authType: "api-key",
            env: {
              GEMINI_API_KEY: "sk-test",
              GEMINI_MODEL: "legacy",
              MY_CUSTOM_VAR: "1",
            },
          },
        }}
      />,
    );

    const extra = container.querySelector<HTMLTextAreaElement>("textarea")!;
    expect(extra.value).toBe("MY_CUSTOM_VAR=1");
    expect(extra.value).not.toContain("GEMINI_MODEL");
  });

  it("获取模型列表走 agy 目录（providersApi.antigravityListModels），下拉按显示名选取", async () => {
    const user = userEvent.setup();
    mocks.antigravityListModels.mockResolvedValue([
      { name: "Gemini 3.8 Flash (Low)", id: "gemini-3.8-flash-low" },
      { name: "Gemini 3.1 Pro (High)", id: "gemini-3.1-pro-high" },
    ]);
    const { container } = render(
      <AntigravityProviderForm
        {...baseProps}
        initialData={{
          name: "Relay",
          settingsConfig: {
            authType: "api-key",
            env: { GEMINI_API_KEY: "sk-test" },
          },
        }}
      />,
    );

    await user.click(screen.getByRole("button", { name: /获取模型列表|fetch/i }));

    await waitFor(() => expect(mocks.antigravityListModels).toHaveBeenCalled());
    await user.click(
      container.querySelector<HTMLButtonElement>(
        "#antigravity-default-model ~ button, button[aria-label*='Select model'], button[role='combobox']",
      ) ?? screen.getAllByRole("button").at(-1)!,
    );

    // 下拉项 = agy 显示名（存储值），选中后写进输入框
    const option = await screen.findByText("Gemini 3.1 Pro (High)");
    await user.click(option);
    const modelInput = container.querySelector<HTMLInputElement>(
      "#antigravity-default-model",
    )!;
    expect(modelInput.value).toBe("Gemini 3.1 Pro (High)");
  });
});
