/**
 * OMP Configuration Utilities
 * 
 * Handles parsing, building, validating, and reconciling OMP configurations.
 * Applies lessons learned from Grok Build diagnostics:
 * - State unification (no drift between different state sources)
 * - Default key reconciliation (auto-correct to valid entries)
 * - Proxy takeover traversal (apply changes to all entries)
 */

import type {
  OmpProviderConfig,
  OmpModelRole,
  OmpSwitchConfig,
  OmpModelsYml,
  OmpValidationError,
  OmpRole,
  ParsedRoleSelector,
  ThinkingLevel,
} from "@/types/omp";

// ────────────────────────────────────────────────────────────────────────────
// Constants
// ────────────────────────────────────────────────────────────────────────────

/**
 * chat 区角色（默认路由到对话模型）
 */
export const OMP_CHAT_ROLES: readonly OmpRole[] = [
  "default",
  "smol",
  "slow",
  "vision",
  "plan",
  "commit",
  "tiny",
  "memory",
  "task",
  "advisor",
] as const;

/**
 * kind 区角色（按模型种类路由：图像 / 搜索 / 语音 / 评审）。
 * 取值常指向 OMP 的合成供应商（`web/parallel`、`local/kokoro`）或非 chat 模型
 * （`openai-codex/gpt-image-1`）。
 */
export const OMP_KIND_ROLES: readonly OmpRole[] = [
  "image",
  "web",
  "speech",
  "dictation",
  "judge",
] as const;

/**
 * OMP 内置角色全集（与 OMP `config/model-roles.ts` 的 MODEL_ROLES 对齐）。
 * 注意这是「内置」集合，不是封闭集合：config.yml 里可以有自定义角色键
 * （OMP 会把 cycleOrder / modelRoles / modelTags 里的新键并入已知角色）。
 */
export const OMP_ROLES: readonly OmpRole[] = [
  ...OMP_CHAT_ROLES,
  ...OMP_KIND_ROLES,
] as const;

export const DEFAULT_OMP_CONFIG: OmpSwitchConfig = {
  version: 1,
  providers: [],
  roles: [],
};

// ────────────────────────────────────────────────────────────────────────────
// Role Selector Parsing
// ────────────────────────────────────────────────────────────────────────────

/**
 * Parse role selector string: "provider/model:thinking-level"
 * Examples:
 * - "anthropic/claude-3.7-sonnet"
 * - "anthropic/claude-3.7-sonnet:high"
 * - "openai/gpt-4o:auto"
 */
export function parseRoleSelector(selector: string): ParsedRoleSelector | null {
  const trimmed = selector.trim();
  if (!trimmed) return null;

  // Split by colon for thinking level
  const [modelPart, thinkingLevel] = trimmed.split(":");
  
  // Split by slash for provider/model
  const slashIndex = modelPart.indexOf("/");
  if (slashIndex === -1) return null;

  const providerId = modelPart.slice(0, slashIndex).trim();
  const modelId = modelPart.slice(slashIndex + 1).trim();

  if (!providerId || !modelId) return null;

  return {
    providerId,
    modelId,
    thinkingLevel: (thinkingLevel?.trim() as ThinkingLevel) || undefined,
  };
}

/**
 * Build role selector string from parts
 */
export function buildRoleSelector(
  providerId: string,
  modelId: string,
  thinkingLevel?: ThinkingLevel,
): string {
  const base = `${providerId}/${modelId}`;
  return thinkingLevel ? `${base}:${thinkingLevel}` : base;
}

// ────────────────────────────────────────────────────────────────────────────
// Configuration Parsing & Building
// ────────────────────────────────────────────────────────────────────────────

/**
 * Parse OMP Switch configuration JSON
 */
export function parseOmpConfig(json: string): OmpSwitchConfig {
  try {
    const parsed = JSON.parse(json);
    return {
      version: parsed.version ?? 1,
      providers: parsed.providers ?? [],
      roles: parsed.roles ?? [],
    };
  } catch (error) {
    console.error("[OmpConfig] Parse error:", error);
    return DEFAULT_OMP_CONFIG;
  }
}

/**
 * Build OMP Switch configuration JSON
 */
export function buildOmpConfig(config: OmpSwitchConfig): string {
  return JSON.stringify(config, null, 2);
}

/**
 * Update OMP configuration with partial changes
 */
export function updateOmpConfig(
  config: OmpSwitchConfig,
  updates: Partial<OmpSwitchConfig>,
): OmpSwitchConfig {
  return {
    ...config,
    ...updates,
  };
}

// ────────────────────────────────────────────────────────────────────────────
// Native OMP Format (models.yml / config.yml)
// ────────────────────────────────────────────────────────────────────────────

/**
 * Export to OMP native models.yml format
 */
export function buildModelsYml(config: OmpSwitchConfig): string {
  const providers: OmpModelsYml["providers"] = {};

  for (const provider of config.providers) {
    if (provider.type === "oauth") {
      // OAuth providers don't go in models.yml
      continue;
    }

    const entry: OmpModelsYml["providers"][string] = {};

    if (provider.type === "api-key" || provider.type === "gateway") {
      entry.baseUrl = provider.baseUrl;
      entry.apiKey = provider.apiKey;
      entry.api = provider.api;
      if (provider.headers) {
        entry.headers = provider.headers;
      }
      if ("authHeader" in provider && provider.authHeader !== undefined) {
        entry.authHeader = provider.authHeader;
      }
    }

    if (provider.type === "local") {
      entry.baseUrl = provider.baseUrl;
      entry.api = provider.api;
    }

    if (provider.models.length > 0) {
      entry.models = provider.models;
    }

    providers[provider.id] = entry;
  }

  // Convert to YAML-like format (simplified)
  const lines = ["providers:"];
  for (const [id, config] of Object.entries(providers)) {
    lines.push(`  ${id}:`);
    if (config.baseUrl) lines.push(`    baseUrl: "${config.baseUrl}"`);
    if (config.apiKey) lines.push(`    apiKey: "${config.apiKey}"`);
    if (config.api) lines.push(`    api: "${config.api}"`);
    if (config.authHeader !== undefined) {
      lines.push(`    authHeader: ${config.authHeader}`);
    }
    if (config.headers) {
      lines.push("    headers:");
      for (const [key, value] of Object.entries(config.headers)) {
        lines.push(`      ${key}: "${value}"`);
      }
    }
    if (config.models && config.models.length > 0) {
      lines.push("    models:");
      for (const model of config.models) {
        lines.push(`      - id: "${model.id}"`);
        lines.push(`        name: "${model.name}"`);
        if (model.api) lines.push(`        api: "${model.api}"`);
        if (model.reasoning) lines.push(`        reasoning: true`);
        lines.push(`        contextWindow: ${model.contextWindow}`);
        lines.push(`        maxTokens: ${model.maxTokens}`);
      }
    }
  }

  return lines.join("\n");
}

/**
 * Export to OMP native config.yml format
 */
export function buildConfigYml(config: OmpSwitchConfig): string {
  const lines = ["# OMP Configuration", ""];

  if (config.roles.length > 0) {
    lines.push("modelRoles:");
    for (const roleMapping of config.roles) {
      const selector = buildRoleSelector(
        roleMapping.providerId,
        roleMapping.modelId,
        roleMapping.thinkingLevel,
      );
      lines.push(`  ${roleMapping.role}: "${selector}"`);
    }
  }

  return lines.join("\n");
}

// ────────────────────────────────────────────────────────────────────────────
// Validation
// ────────────────────────────────────────────────────────────────────────────

/**
 * Validate OMP configuration
 */
export function validateOmpConfig(
  config: OmpSwitchConfig,
): OmpValidationError[] {
  const errors: OmpValidationError[] = [];

  // Check version
  if (config.version !== 1) {
    errors.push({
      field: "version",
      message: `Unsupported version: ${config.version}`,
    });
  }

  // Check providers
  if (!Array.isArray(config.providers)) {
    errors.push({
      field: "providers",
      message: "Providers must be an array",
    });
  } else {
    const ids = new Set<string>();
    for (const [index, provider] of config.providers.entries()) {
      const prefix = `providers[${index}]`;

      if (!provider.id || typeof provider.id !== "string") {
        errors.push({
          field: `${prefix}.id`,
          message: "Provider ID is required",
        });
      } else if (ids.has(provider.id)) {
        errors.push({
          field: `${prefix}.id`,
          message: `Duplicate provider ID: ${provider.id}`,
        });
      } else {
        ids.add(provider.id);
      }

      if (!provider.name) {
        errors.push({
          field: `${prefix}.name`,
          message: "Provider name is required",
        });
      }

      if (!provider.type || !["oauth", "api-key", "gateway", "local"].includes(provider.type)) {
        errors.push({
          field: `${prefix}.type`,
          message: `Invalid provider type: ${provider.type}`,
        });
      }

      if (provider.type === "oauth") {
        if (!("oauthProviderId" in provider) || !provider.oauthProviderId) {
          errors.push({
            field: `${prefix}.oauthProviderId`,
            message: "OAuth provider ID is required for OAuth providers",
          });
        }
      }

      if (provider.type === "api-key" || provider.type === "gateway") {
        if (!("baseUrl" in provider) || !provider.baseUrl) {
          errors.push({
            field: `${prefix}.baseUrl`,
            message: "Base URL is required for API key/gateway providers",
          });
        }
        if (!("apiKey" in provider) || !provider.apiKey) {
          errors.push({
            field: `${prefix}.apiKey`,
            message: "API key is required for API key/gateway providers",
          });
        }
      }

      if (provider.type === "local") {
        if (!("baseUrl" in provider) || !provider.baseUrl) {
          errors.push({
            field: `${prefix}.baseUrl`,
            message: "Base URL is required for local providers",
          });
        }
      }

      if (!Array.isArray(provider.models)) {
        errors.push({
          field: `${prefix}.models`,
          message: "Provider models must be an array",
        });
      }
    }
  }

  // Check roles
  if (!Array.isArray(config.roles)) {
    errors.push({
      field: "roles",
      message: "Roles must be an array",
    });
  } else {
    const assignedRoles = new Set<string>();
    for (const [index, roleMapping] of config.roles.entries()) {
      const prefix = `roles[${index}]`;

      // 角色名不校验是否属于内置集合：OMP 支持自定义角色键（cycleOrder /
      // modelRoles / modelTags 里的新名字都会被并入），只要求非空且不重复。
      if (!roleMapping.role) {
        errors.push({
          field: `${prefix}.role`,
          message: "Role name is required",
        });
      } else if (assignedRoles.has(roleMapping.role)) {
        errors.push({
          field: `${prefix}.role`,
          message: `Duplicate role assignment: ${roleMapping.role}`,
        });
      } else {
        assignedRoles.add(roleMapping.role);
      }

      if (!roleMapping.providerId) {
        errors.push({
          field: `${prefix}.providerId`,
          message: "Provider ID is required",
        });
      }

      if (!roleMapping.modelId) {
        errors.push({
          field: `${prefix}.modelId`,
          message: "Model ID is required",
        });
      }
    }
  }

  return errors;
}

/**
 * Check if configuration is valid
 */
export function isValidOmpConfig(config: OmpSwitchConfig): boolean {
  return validateOmpConfig(config).length === 0;
}

// ────────────────────────────────────────────────────────────────────────────
// State Reconciliation (Grok Build lessons applied)
// ────────────────────────────────────────────────────────────────────────────

/**
 * Reconcile role assignments to ensure all point to existing providers/models
 * Returns corrected roles array
 */
export function reconcileRoles(
  roles: OmpModelRole[],
  providers: OmpProviderConfig[],
): OmpModelRole[] {
  const validProviderIds = new Set(providers.map((p) => p.id));
  
  return roles.filter((role) => {
    // Check if provider exists
    if (!validProviderIds.has(role.providerId)) {
      console.warn(
        `[OmpConfig] Role ${role.role} points to non-existent provider: ${role.providerId}`,
      );
      return false;
    }

    // Check if model exists in that provider
    const provider = providers.find((p) => p.id === role.providerId);
    if (!provider) return false;

    const modelExists = provider.models.some((m) => m.id === role.modelId);
    if (!modelExists) {
      console.warn(
        `[OmpConfig] Role ${role.role} points to non-existent model: ${role.modelId} in provider ${role.providerId}`,
      );
      return false;
    }

    return true;
  });
}

/**
 * Get role assignment for a specific role
 */
export function getRoleAssignment(
  config: OmpSwitchConfig,
  role: string,
): OmpModelRole | undefined {
  return config.roles.find((r) => r.role === role);
}

/**
 * Set role assignment (returns new config)
 */
export function setRoleAssignment(
  config: OmpSwitchConfig,
  role: string,
  providerId: string,
  modelId: string,
  thinkingLevel?: ThinkingLevel,
): OmpSwitchConfig {
  const existingIndex = config.roles.findIndex((r) => r.role === role);
  
  const newRoleMapping: OmpModelRole = {
    role,
    providerId,
    modelId,
    thinkingLevel,
  };

  const roles = [...config.roles];
  if (existingIndex >= 0) {
    roles[existingIndex] = newRoleMapping;
  } else {
    roles.push(newRoleMapping);
  }

  return {
    ...config,
    roles,
  };
}

/**
 * Remove role assignment (returns new config)
 */
export function removeRoleAssignment(
  config: OmpSwitchConfig,
  role: string,
): OmpSwitchConfig {
  return {
    ...config,
    roles: config.roles.filter((r) => r.role !== role),
  };
}
