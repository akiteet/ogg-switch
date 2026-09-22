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
 * 本机中转站（loopback + 密钥）是真实场景：models.yml 不存 type，后端按 baseUrl
 * 推断（127.0.0.1 → local）。编辑表单曾按 type 决定是否回填 apiKey / headers /
 * authHeader，导致 key 显示为空、无法拉取模型，保存还会清掉这些字段。
 */
const storedLoopbackRelay = {
  id: "workbuddy",
  name: "WorkBuddy",
  type: "local",
  category: "local",
  models: [
    { id: "cn:hy3", name: "Hy3", contextWindow: 188000, maxTokens: 63000 },
  ],
  baseUrl: "http://127.0.0.1:7864/v1",
  apiKey: "wbk_hidden_key",
  api: "openai-completions",
  headers: { "X-Test": "1" },
  authHeader: false,
};

describe("OmpProviderForm 编辑态回填", () => {
  it("推断类型为 local 的本机中转站仍然回填 apiKey / headers / authHeader", () => {
    const { container } = render(
      <OmpProviderForm
        submitLabel="保存"
        onSubmit={() => {}}
        onCancel={() => {}}
        providerId="workbuddy"
        initialData={{
          name: "WorkBuddy",
          websiteUrl: "http://127.0.0.1:7864",
          settingsConfig: { config: storedLoopbackRelay },
        }}
      />,
    );

    // API Key 输入框（password 类型）带回原值
    const apiKeyInput = container.querySelector<HTMLInputElement>(
      'input[type="password"][placeholder^="sk-"]',
    );
    expect(apiKeyInput?.value).toBe("wbk_hidden_key");

    // Base URL 照旧回填
    const baseUrlInput = container.querySelector<HTMLInputElement>(
      'input[type="url"]',
    );
    expect(baseUrlInput?.value).toBe("http://127.0.0.1:7864/v1");

    // 自定义 headers 不被清空
    const headersArea = container.querySelector<HTMLTextAreaElement>("textarea");
    expect(headersArea?.value).toContain("X-Test");

    // authHeader=false 不被改回 true
    const authHeader = container.querySelector<HTMLInputElement>("#authHeader");
    expect(authHeader?.checked).toBe(false);
  });

  it("保存时把 apiKey / headers / authHeader 原样写回（type 归一为 api-key）", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(
      <OmpProviderForm
        submitLabel="保存"
        onSubmit={onSubmit}
        onCancel={() => {}}
        providerId="workbuddy"
        initialData={{
          name: "WorkBuddy",
          settingsConfig: { config: storedLoopbackRelay },
        }}
      />,
    );

    await user.click(screen.getByRole("button", { name: "保存" }));

    expect(mocks.saveOmpProvider).toHaveBeenCalledTimes(1);
    const saved = mocks.saveOmpProvider.mock.calls[0]![0] as Record<
      string,
      unknown
    >;
    expect(saved.type).toBe("api-key");
    expect(saved.apiKey).toBe("wbk_hidden_key");
    expect(saved.headers).toEqual({ "X-Test": "1" });
    expect(saved.authHeader).toBe(false);
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });
});

describe("OmpProviderForm 无密钥的本地供应商", () => {
  it("loopback + 无密钥保存为 local（否则会被 api-key 校验拦死）", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    const { container } = render(
      <OmpProviderForm
        submitLabel="保存"
        onSubmit={onSubmit}
        onCancel={() => {}}
      />,
    );

    const nameInput = container.querySelector<HTMLInputElement>(
      'input[name="name"]',
    )!;
    await user.type(nameInput, "Ollama Local");

    const baseUrlInput =
      container.querySelector<HTMLInputElement>('input[type="url"]')!;
    fireEvent.change(baseUrlInput, {
      target: { value: "http://localhost:11434/v1" },
    });

    await user.click(screen.getByRole("button", { name: "保存" }));

    expect(mocks.saveOmpProvider).toHaveBeenCalledTimes(1);
    const saved = mocks.saveOmpProvider.mock.calls[0]![0] as Record<
      string,
      unknown
    >;
    expect(saved.type).toBe("local");
    expect(saved.category).toBe("local");
    expect(saved.baseUrl).toBe("http://localhost:11434/v1");
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("远程地址缺少密钥时仍然拦截（不能静默存成 local）", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    const { container } = render(
      <OmpProviderForm
        submitLabel="保存"
        onSubmit={onSubmit}
        onCancel={() => {}}
      />,
    );

    const nameInput = container.querySelector<HTMLInputElement>(
      'input[name="name"]',
    )!;
    await user.type(nameInput, "Remote Relay");

    const baseUrlInput =
      container.querySelector<HTMLInputElement>('input[type="url"]')!;
    fireEvent.change(baseUrlInput, {
      target: { value: "https://relay.example.com/v1" },
    });

    await user.click(screen.getByRole("button", { name: "保存" }));

    expect(mocks.saveOmpProvider).not.toHaveBeenCalled();
    expect(onSubmit).not.toHaveBeenCalled();
  });
});
