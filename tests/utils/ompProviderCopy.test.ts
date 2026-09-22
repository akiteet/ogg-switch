import { describe, expect, it } from "vitest";
import {
  buildOmpProviderCopy,
  parseEmbeddedOmpProvider,
} from "@/utils/ompProviderCopy";
import type { OmpApiKeyProvider, OmpOAuthProvider } from "@/types/omp";

const original: OmpApiKeyProvider = {
  id: "workbuddy",
  name: "WorkBuddy",
  type: "api-key",
  category: "api",
  models: [
    { id: "cn:hy3", name: "Hy3", contextWindow: 188000, maxTokens: 63000 },
  ],
  baseUrl: "http://127.0.0.1:7864/v1",
  apiKey: "wbk_test",
  api: "openai-completions",
  headers: { "X-Test": "1" },
  authHeader: false,
};

const settingsConfig = { config: JSON.stringify(original) };

describe("buildOmpProviderCopy", () => {
  it("改写内嵌配置的 id / name，其余字段保真", () => {
    const copy = buildOmpProviderCopy(settingsConfig, "workbuddy-copy", "WorkBuddy copy");
    expect(copy).not.toBeNull();
    expect(copy!.provider).toMatchObject({
      id: "workbuddy-copy",
      name: "WorkBuddy copy",
      // 凭据 / 模型 / 协议 / 自定义头原样保留
      baseUrl: original.baseUrl,
      apiKey: "wbk_test",
      authHeader: false,
      headers: { "X-Test": "1" },
      models: original.models,
    });
    // settingsConfig 与 provider 同步（走通用 Provider 载荷时需要）
    expect(parseEmbeddedOmpProvider(copy!.settingsConfig)?.id).toBe("workbuddy-copy");
    // 原对象不被改动
    expect(original.id).toBe("workbuddy");
  });

  it("OAuth 供应商沿用同一 oauthProviderId", () => {
    const oauth: OmpOAuthProvider = {
      id: "openai-codex",
      name: "openai-codex",
      type: "oauth",
      category: "subscription",
      models: [],
      oauthProviderId: "openai-codex",
      api: "openai-completions",
    };
    const copy = buildOmpProviderCopy(
      { config: JSON.stringify(oauth) },
      "openai-codex-copy",
      "openai-codex copy",
    );
    expect(copy!.provider).toMatchObject({
      type: "oauth",
      oauthProviderId: "openai-codex",
    });
  });

  it("接受对象形态的 config", () => {
    const copy = buildOmpProviderCopy({ config: original }, "x-copy", "x copy");
    expect(copy!.provider.id).toBe("x-copy");
  });

  it("配置无法解析 / 缺 id 时返回 null（调用方应报错中止）", () => {
    expect(buildOmpProviderCopy({ config: "{ not json" }, "a-copy", "a copy")).toBeNull();
    expect(buildOmpProviderCopy(undefined, "a-copy", "a copy")).toBeNull();
    expect(buildOmpProviderCopy(settingsConfig, "", "a copy")).toBeNull();
  });
});
