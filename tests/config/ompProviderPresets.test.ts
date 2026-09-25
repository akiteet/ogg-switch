import { describe, expect, it } from "vitest";
import { ALL_OMP_PRESETS } from "@/config/ompProviderPresets";

/**
 * 内置 / 常见分层快照（2026-09-24，依据 can1357/oh-my-pi@740f3e3 的
 * docs/providers.md：oauth 清单 + 环境变量表 + 本地引擎表逐条比对）。
 *
 * 分层只划分「omp 内置 vs OGG 增补」；OAuth 成员以 omp 源码
 * packages/catalog/src/compat/rules/auth/<id>.kdl 为唯一权威，不参与调整。
 */
const COMMON_IDS = [
  // 国内官方 API（providers.md 未收录）
  "cohere-api",
  "perplexity-api",
  "zhipu",
  "baichuan",
  "01ai",
  "doubao",
  "qwen",
  "hunyuan",
  "replicate",
  "anyscale",
  // 聚合站 / 中转站模板
  "modelscope",
  "one-api",
  "new-api",
  "fastgpt",
  "runpod",
  "infermatic",
  "lepton",
  "featherless",
  "together-xyz",
  "octo",
  "unify",
  "portkey",
  "helicone",
  "langfuse",
  "nebius",
  "lambda",
  "modal",
].sort();

describe("OMP 预设分层（builtin / common）", () => {
  it("常见供应商分层精确落在预期成员上", () => {
    const common = ALL_OMP_PRESETS
      .filter((preset) => preset.tier === "common")
      .map((preset) => preset.id)
      .sort();
    expect(common).toEqual(COMMON_IDS);
  });

  it("hunyuan / zhipu / one-api 归常见供应商", () => {
    for (const id of ["hunyuan", "zhipu", "one-api", "new-api"]) {
      const matches = ALL_OMP_PRESETS.filter((preset) => preset.id === id);
      expect(matches.length, id).toBeGreaterThan(0);
      for (const preset of matches) {
        expect(preset.tier, id).toBe("common");
      }
    }
  });

  it("deepseek / moonshot / openrouter / siliconflow 保持内置", () => {
    for (const id of ["deepseek", "moonshot", "openrouter", "siliconflow"]) {
      const matches = ALL_OMP_PRESETS.filter((preset) => preset.id === id);
      expect(matches.length, id).toBeGreaterThan(0);
      for (const preset of matches) {
        expect(preset.tier, id).toBe("builtin");
      }
    }
  });

  it("omp 内置的 commandcode / abliteration 已收录为 chat 预设（builtin）", () => {
    // 2026-09-25 对照 omp 源码 auth 规则补齐：两供应商均为 api-key 登录的 chat 供应商。
    // exa/tavily/kagi/parallel/typesafe（搜索/judge 工具）、stencil（omp 服务凭据）、
    // apple（macOS 本机）不是 chat 供应商，刻意不收录。
    for (const id of ["commandcode", "abliteration"]) {
      const matches = ALL_OMP_PRESETS.filter((preset) => preset.id === id);
      expect(matches.length, id).toBe(1);
      const preset = matches[0]!;
      expect(preset.type, id).toBe("api-key");
      expect(preset.tier, id).toBe("builtin");
      expect(preset.defaultBaseUrl, id).toBeTruthy();
    }
  });

  it("OAuth 预设全部为 builtin（kdl 权威，不参与分层调整）", () => {
    const oauthPresets = ALL_OMP_PRESETS.filter(
      (preset) => preset.type === "oauth",
    );
    expect(oauthPresets.length).toBe(20);
    for (const preset of oauthPresets) {
      expect(preset.tier, preset.id).toBe("builtin");
    }
  });

  it("合并后的每个预设都带 tier 标记且总数不变", () => {
    expect(ALL_OMP_PRESETS.length).toBe(114);
    for (const preset of ALL_OMP_PRESETS) {
      expect(["builtin", "common"], preset.id).toContain(preset.tier);
    }
  });
});
