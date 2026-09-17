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
 */
export type OmpProviderCategory = "subscription" | "api" | "gateway" | "local";

/**
 * API protocol for model communication
 */
export type OmpApiProtocol =
  | "openai-completions"
  | "openai-responses"
  | "anthropic-messages"
  | "google-generative-ai";

// ────────────────────────────────────────────────────────────────────────────
// Model Roles (10 fixed semantic roles)
// ────────────────────────────────────────────────────────────────────────────

/**
 * OMP's 10 managed model roles
 * Each role maps to a specific provider/model combination
 */
export type OmpRole =
  | "default"   // Default general-purpose model
  | "smol"      // Fast, cheap model for simple tasks
  | "slow"      // High-quality, slow model for complex tasks
  | "plan"      // Planning and architecture tasks
  | "commit"    // Git commit message generation
  | "vision"    // Visual/image understanding
  | "designer"  // Design-related tasks
  | "task"      // Background task execution
  | "advisor"   // Advisory/consulting tasks
  | "tiny";     // Extremely lightweight model

/**
 * Thinking level for extended thinking models
 */
export type ThinkingLevel =
  | "off"       // No thinking
  | "minimal"   // Minimal thinking
  | "low"       // Low thinking
  | "medium"    // Medium thinking
  | "high"      // High thinking
  | "xhigh"     // Extra high thinking
  | "max"       // Maximum thinking
  | "auto";     // Automatic thinking level

/**
 * Model role mapping configuration
 * Format: "provider/model:thinking-level"
 * Example: "anthropic/claude-3.7-sonnet:high"
 */
export interface OmpModelRole {
  role: OmpRole;
  providerId: string;           // "anthropic", "openai", etc.
  modelId: string;              // "claude-3.7-sonnet", "gpt-4o", etc.
  thinkingLevel?: ThinkingLevel; // Optional thinking level
}

// ────────────────────────────────────────────────────────────────────────────
// Model Information
// ────────────────────────────────────────────────────────────────────────────

/**
 * Model information (from models.yml)
 */
export interface OmpModelInfo {
  id: string;                   // Model ID
  name: string;                 // Display name
  api?: OmpApiProtocol;        // Per-model API override
  reasoning?: boolean;          // Supports extended thinking/reasoning
  contextWindow: number;        // Context window size
  maxTokens: number;           // Maximum output tokens
}

// ────────────────────────────────────────────────────────────────────────────
// Provider Configuration
// ────────────────────────────────────────────────────────────────────────────

/**
 * Base provider configuration
 */
interface OmpProviderBase {
  id: string;                   // Provider ID (unique)
  name: string;                 // Display name
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
}

/**
 * OAuth provider configuration
 * Uses OMP's auth-broker for authentication
 */
export interface OmpOAuthProvider extends OmpProviderBase {
  type: "oauth";
  oauthProviderId: string;      // OMP auth-broker provider ID
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
  apiKey: string;               // Can be env var name or secret-get command
  api: OmpApiProtocol;
  headers?: Record<string, string>;
  authHeader?: boolean;         // Whether to send Authorization header
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
  baseUrl: string;              // e.g., "http://localhost:11434" for Ollama
  api: "openai-completions";    // Local servers typically use OpenAI API
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
  envKeyName?: string;          // e.g., "ANTHROPIC_API_KEY"
  // Preset models (optional)
  defaultModels?: OmpModelInfo[];
}

// ────────────────────────────────────────────────────────────────────────────
// Configuration Files
// ────────────────────────────────────────────────────────────────────────────

/**
 * models.yml structure (OMP native format)
 */
export interface OmpModelsYml {
  providers: Record<string, {
    baseUrl?: string;
    apiKey?: string;
    api?: OmpApiProtocol;
    authHeader?: boolean;
    headers?: Record<string, string>;
    models?: OmpModelInfo[];
  }>;
}

/**
 * config.yml structure (OMP native format)
 */
export interface OmpConfigYml {
  modelRoles?: Record<OmpRole, string>;  // role -> "provider/model:thinking"
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
  expiresAt?: number;  // Unix timestamp
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
