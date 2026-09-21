/**
 * Antigravity CLI (agy) 预设供应商配置模板
 *
 * agy 与 Gemini CLI 的配置机制完全不同（官方文档实锤）：
 * - 不读取 ~/.gemini/.env；
 * - API key 模式 = ~/.gemini/antigravity-cli/settings.json 的
 *   `modelProvider: "gemini"` + 持久环境变量 GEMINI_API_KEY /
 *   GOOGLE_GEMINI_BASE_URL（Gemini 兼容端点），缺一 CLI 无法启动；
 * - Google OAuth 登录态在 antigravity-oauth-token 文件，官方条目不接管该文件。
 *
 * 收录规则：v1 仅官方两条，第三方中转站（需提供 Gemini 兼容端点）走自定义添加。
 */
import type { ProviderCategory } from "@/types";
import type { PresetTheme } from "./claudeProviderPresets";
import { ANTIGRAVITY_OFFICIAL_PROVIDER_ID } from "@/utils/providerCapabilities";

export type AntigravityAuthType = "oauth" | "api-key";

export interface AntigravityProviderPreset {
  name: string;
  nameKey?: string; // i18n key for localized display name
  websiteUrl: string;
  apiKeyUrl?: string;
  authType: AntigravityAuthType;
  /** api-key 预设的 env 模板（key 留空待用户填写） */
  env?: Record<string, string>;
  description?: string;
  category?: ProviderCategory;
  endpointCandidates?: string[];
  isPartner?: boolean; // 与其他预设对齐的占位（antigravity v1 无合作站）
  theme?: PresetTheme;
  icon?: string;
  iconColor?: string;
  primePartner?: boolean; // 与其他预设对齐的占位（antigravity v1 不使用）
}

// 官方条目与后端 seed（providers_seed.rs 的 "Antigravity Official"）对应：
// oauth 无 token = 摘掉 modelProvider + 清 API key 环境变量，
// 不接管 agy 自身的 Google 登录态（token 文件原样保留）。
// 预设 id 复用固定 provider id，AddProviderDialog 据此走 ensure seed 流程。
export const antigravityOfficialPreset: AntigravityProviderPreset = {
  name: "Antigravity Official",
  websiteUrl: "https://antigravity.google/",
  authType: "oauth",
  category: "official",
  icon: "antigravity",
  iconColor: "currentColor",
};

export const antigravityProviderPresets: AntigravityProviderPreset[] = [
  antigravityOfficialPreset,
  {
    name: "Google Gemini API",
    websiteUrl: "https://ai.google.dev/",
    apiKeyUrl: "https://aistudio.google.com/apikey",
    authType: "api-key",
    env: {
      GEMINI_API_KEY: "",
    },
    category: "official",
    icon: "antigravity",
    iconColor: "#4285F4",
  },
];

export function isAntigravityOfficialPresetId(presetId?: string): boolean {
  return presetId === ANTIGRAVITY_OFFICIAL_PROVIDER_ID;
}
