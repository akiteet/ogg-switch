import { describe, expect, it } from "vitest";
import {
  antigravityOfficialPreset,
  antigravityProviderPresets,
  isAntigravityOfficialPresetId,
} from "./antigravityProviderPresets";
import { ANTIGRAVITY_OFFICIAL_PROVIDER_ID } from "../utils/providerCapabilities";

describe("antigravityProviderPresets", () => {
  it("has unique preset names", () => {
    const names = antigravityProviderPresets.map((p) => p.name);
    expect(new Set(names).size).toBe(names.length);
  });

  it("official preset maps to the backend seed id", () => {
    expect(antigravityProviderPresets[0]).toBe(antigravityOfficialPreset);
    expect(antigravityOfficialPreset.authType).toBe("oauth");
    expect(antigravityOfficialPreset.category).toBe("official");
    expect(
      isAntigravityOfficialPresetId(ANTIGRAVITY_OFFICIAL_PROVIDER_ID),
    ).toBe(true);
    // v1 语义：官方条目不携带 token 快照（不接管 agy 登录态文件）
    expect(antigravityOfficialPreset).not.toHaveProperty("token");
    expect(antigravityOfficialPreset).not.toHaveProperty("env");
  });

  it("api-key presets always carry a GEMINI_API_KEY env template", () => {
    for (const preset of antigravityProviderPresets) {
      if (preset.authType !== "api-key") continue;
      expect(preset.env, preset.name).toBeDefined();
      expect("GEMINI_API_KEY" in (preset.env ?? {}), preset.name).toBe(true);
    }
  });

  it("api-key preset does not set a custom endpoint by default", () => {
    const geminiApi = antigravityProviderPresets.find(
      (p) => p.name === "Google Gemini API",
    );
    expect(geminiApi).toBeDefined();
    expect(geminiApi?.env?.GOOGLE_GEMINI_BASE_URL).toBeUndefined();
  });
});
