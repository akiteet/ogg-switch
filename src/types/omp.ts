import type { UsageScript } from "../types";
/**
 * Oh My Pi (OMP) Type Definitions
 * Based on OMP v18+ configuration format
 */

// ────────────────────────────────────────────────────────────────────────────
// Provider Types
// ────────────────────────────────────────────────────────────────────────────

/**
 * Provider type classification
 */
export type OmpProviderType = "oauth" | "api-key" | "gateway" | "local";

/**
 * Provider category for UI grouping
 *
 * - `subscription`：OAuth 登录的 omp 内置供应商（凭据库 /login）
 * - `api`：omp 内置 API Key 供应商（providers.md 环境变量表内的官方/聚合条目）
 * - `common`：常见供应商——OGG 增补预设，不在 omp 官方 providers.md 清单里
 *   （如腾讯混元、豆包、one-api/new-api 模板）
 * - `custom`：用户自建的非预设供应商
 * - `gateway` / `local`：legacy 与本地引擎（`gateway` 仅旧数据读取兼容，新数据不再产生）
 */
export type OmpProviderCategory =
  | "subscription"
  | "api"
  | "common"
  | "custom"
  | "gateway"
  | "local";

/**
 * API protocol for model communication
 */
export type OmpApiProtocol =
  | "openai-completions"
  | "openai-responses"
  | "anthropic-messages"
  | "google-generative-ai";

// ────────────────────────────────────────────────────────────────────────────
// Model Roles
//
// 与 OMP 18.x 内置角色表（`config/model-roles.ts` 的 MODEL_ROLES）一致：
// chat 区 10 个 + kind 区 5 个。角色名不是封闭集合——config.yml 里出现的自定义键
// （如已废弃的 designer）也是合法角色，因此运行时用的是任意字符串。
// ────────────────────────────────────────────────────────────────────────────

/**
 * OMP 内置角色 id
 */
export type OmpRole =
  // chat 区
  | "default" // 通用模型
  | "smol" // 快速便宜的模型（Fast）
  | "slow" // 高质量推理模型（Thinking）
  | "vision" // 图像理解
  | "plan" // 规划/架构（Architect）
  | "commit" // Git 提交消息生成
  | "tiny" // 极轻量任务
  | "memory" // 记忆/历史压缩
  | "task" // 子任务（Subtask）
  | "advisor" // 咨询建议
  // kind 区（按模型种类路由）
  | "image" // 图像生成
  | "web" // 联网搜索
  | "speech" // 语音合成（tts）
  | "dictation" // 语音识别（stt）
  | "judge"; // 评审/判定

/**
 * Thinking level for extended thinking models
 */
export type ThinkingLevel =
  | "off" // No thinking
  | "minimal" // Minimal thinking
  | "low" // Low thinking
  | "medium" // Medium thinking
  | "high" // High thinking
  | "xhigh" // Extra high thinking
  | "max" // Maximum thinking
  | "auto"; // Automatic thinking level

/**
 * Model role mapping configuration
 * Format: "provider/model:thinking-level"
 * Example: "anthropic/claude-3.7-sonnet:high"
 */
export interface OmpModelRole {
  /** 角色名：内置 id 或 config.yml 里的自定义键 */
  role: string;
  providerId: string; // "anthropic", "openai", "web", "local" …
  modelId: string; // "claude-3.7-sonnet", "gpt-4o", "parallel", "kokoro" …
  thinkingLevel?: ThinkingLevel; // Optional thinking level
}

// ────────────────────────────────────────────────────────────────────────────
// Model Information
// ────────────────────────────────────────────────────────────────────────────

/**
 * Model information (from models.yml)
 */
export interface OmpModelInfo {
  id: string; // Model ID
  name: string; // Display name
  api?: OmpApiProtocol; // Per-model API override
  reasoning?: boolean; // Supports extended thinking/reasoning
  contextWindow: number; // Context window size
  maxTokens: number; // Maximum output tokens
  /**
   * OMP 目录里的模型种类（chat / tiny / image / tts / stt / search / judge /
   * embedding / rerank）。仅 `omp models --json` 会带回；models.yml 不存该字段。
   */
  kind?: string;
}

// ────────────────────────────────────────────────────────────────────────────
// Provider Configuration
// ────────────────────────────────────────────────────────────────────────────

/**
 * Base provider configuration
 */
interface OmpProviderBase {
  id: string; // Provider ID (unique)
  name: string; // Display name
  type: OmpProviderType;
  category: OmpProviderCategory;
  description?: string;
  websiteUrl?: string;
  docsUrl?: string;
  icon?: string;
  models: OmpModelInfo[];
  // OGG meta store 的拖拽排序值（仅列表展示用，不写 models.yml）
  sortIndex?: number | null;
  // 库模式成员标记：true = 已在配置（显示「移除」）；false = 仅存于库（显示「添加」）。
  // 由后端 load 时按条目来源计算，不写 models.yml
  inConfig?: boolean;
  // 用量查询脚本配置（真源在 OGG meta store，仅 GUI 传输层，不写 models.yml）。
  // 键名必须与后端 `OmpProviderConfig`（`#[serde(rename_all = "camelCase")]`）一致：
  // camelCase 的 `usageScript`。历史上这里写成 snake_case，导致前端永远读不到脚本、
  // 卡片用量区恒不渲染（v1.1.3 修正）。
  usageScript?: UsageScript | null;
}

/**
 * OAuth provider configuration
 * Uses OMP's auth-broker for authentication
 */
export interface OmpOAuthProvider extends OmpProviderBase {
  type: "oauth";
  oauthProviderId: string; // OMP auth-broker provider ID
  api: OmpApiProtocol;
  // OAuth login state (runtime)
  isLoggedIn?: boolean;
  accounts?: OAuthAccount[];
}

/**
 * OAuth account information
 */
export interface OAuthAccount {
  index: number;
  email?: string;
  accountName?: string;
  expiresAt?: string;
}

/**
 * API Key provider configuration
 * Uses API keys for authentication
 */
export interface OmpApiKeyProvider extends OmpProviderBase {
  type: "api-key";
  baseUrl: string;
  apiKey: string; // Can be env var name or secret-get command
  api: OmpApiProtocol;
  headers?: Record<string, string>;
  authHeader?: boolean; // Whether to send Authorization header
}

/**
 * Gateway/Aggregator provider configuration
 */
export interface OmpGatewayProvider extends OmpProviderBase {
  type: "gateway";
  baseUrl: string;
  apiKey: string;
  api: OmpApiProtocol;
  headers?: Record<string, string>;
}

/**
 * Local inference provider configuration
 */
export interface OmpLocalProvider extends OmpProviderBase {
  type: "local";
  baseUrl: string; // e.g., "http://localhost:11434" for Ollama
  api: OmpApiProtocol;
  // models.yml 允许本地条目也带 headers / authHeader（部分本地服务需要自定义头），
  // 编辑保存时必须原样带回，否则等于静默清空用户配置。
  headers?: Record<string, string>;
  authHeader?: boolean;
}

/**
 * Union type for all provider configurations
 */
export type OmpProviderConfig =
  | OmpOAuthProvider
  | OmpApiKeyProvider
  | OmpGatewayProvider
  | OmpLocalProvider;

// ────────────────────────────────────────────────────────────────────────────
// Provider Presets
// ────────────────────────────────────────────────────────────────────────────

/**
 * Provider preset definition (for UI)
 */
export interface OmpProviderPreset {
  id: string;
  name: string;
  type: OmpProviderType;
  category: OmpProviderCategory;
  description: string;
  websiteUrl: string;
  docsUrl?: string;
  icon?: string;
  // OAuth-specific
  oauthProviderId?: string;
  // API Key-specific
  defaultBaseUrl?: string;
  defaultApi?: OmpApiProtocol;
  requiresApiKey?: boolean;
  envKeyName?: string; // e.g., "ANTHROPIC_API_KEY"
  // Preset models (optional)
  defaultModels?: OmpModelInfo[];
  /**
   * 内置/常见分层（合并 ALL_OMP_PRESETS 时统一打上）：
   * - `builtin`：omp 官方 providers.md 列出的内置供应商（OAuth 组以 omp 源码 kdl 为准）
   * - `common`：OGG 增补的常见供应商（providers.md 未收录，如腾讯混元、聚合站模板）
   */
  tier?: "builtin" | "common";
}

// ────────────────────────────────────────────────────────────────────────────
// Configuration Files
// ────────────────────────────────────────────────────────────────────────────

/**
 * models.yml structure (OMP native format)
 */
export interface OmpModelsYml {
  providers: Record<
    string,
    {
      baseUrl?: string;
      apiKey?: string;
      api?: OmpApiProtocol;
      authHeader?: boolean;
      headers?: Record<string, string>;
      models?: OmpModelInfo[];
    }
  >;
}

/**
 * config.yml structure (OMP native format)
 */
export interface OmpConfigYml {
  modelRoles?: Record<string, string>; // role -> "provider/model:thinking"
  // ... other config fields
}

/**
 * OGG Switch's OMP configuration storage
 * (stored in ~/.config/oggswitch/omp_*.json)
 */
export interface OmpSwitchConfig {
  version: number;
  providers: OmpProviderConfig[];
  roles: OmpModelRole[];
}

// ────────────────────────────────────────────────────────────────────────────
// Utility Types
// ────────────────────────────────────────────────────────────────────────────

/**
 * Role selector string: "provider/model:thinking-level"
 */
export type RoleSelector = string;

/**
 * Parse result for role selector
 */
export interface ParsedRoleSelector {
  providerId: string;
  modelId: string;
  thinkingLevel?: ThinkingLevel;
}

/**
 * Import/Export result
 */
export interface OmpImportResult {
  providersImported: number;
  rolesImported: number;
  errors: string[];
}

/**
 * Validation error
 */
export interface OmpValidationError {
  field: string;
  message: string;
}

/**
 * OAuth authentication status
 */
export interface OmpOAuthStatus {
  isAuthenticated: boolean;
  providerId: string;
  accountInfo?: {
    email?: string;
    username?: string;
    displayName?: string;
  };
  expiresAt?: number; // Unix timestamp
}

/**
 * Live detection result for the machine's own Oh My Pi install.
 * Reported by the backend so the UI can say "not installed" instead of
 * silently rendering an empty list.
 */
export interface OmpLiveStatus {
  detected: boolean;
  agentDir: string;
  modelsPath?: string;
  configPath?: string;
  cliAvailable: boolean;
  message: string;
}

/**
 * OMP 目录里「已启用」的供应商（`omp models --json --kind all` 的去重结果）。
 * 含 models.yml 之外的合成供应商：`web`（联网搜索后端池）、`local`（本机 tts/stt）、
 * 以及各类 OAuth 供应商——角色选择器需要它们。
 */
export interface OmpEnabledProvider {
  id: string;
  modelCount: number;
}
