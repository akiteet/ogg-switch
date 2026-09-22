import { invoke } from "@tauri-apps/api/core";
import type {
  OmpProviderConfig,
  OmpModelRole,
  OmpSwitchConfig,
} from "@/types/omp";

/**
 * Oh My Pi (OMP) API
 *
 * The source of truth is the machine's own OMP config:
 *   ~/.omp/agent/models.yml + ~/.omp/agent/config.yml
 * Reads and writes go straight to those files (atomic replace), not a private
 * shadow copy. If the directory does not exist, nothing is invented.
 */

// ── Config ─────────────────────────────────────────────────────────────────

/** Read providers + roles from ~/.omp/agent. */
export async function readOmpConfig(): Promise<OmpSwitchConfig> {
  return invoke<OmpSwitchConfig>("read_omp_config");
}

// ── Providers ──────────────────────────────────────────────────────────────

export async function saveOmpProvider(
  provider: OmpProviderConfig,
): Promise<void> {
  return invoke("save_omp_provider", { provider });
}

/**
 * 只写 OGG 的供应商库（不写 models.yml）：副本默认「未添加」，用户确认后再
 * `saveOmpProvider` 进 live。用于「复制供应商」。
 */
export async function saveOmpProviderToLibrary(
  provider: OmpProviderConfig,
): Promise<void> {
  return invoke("save_omp_provider_to_library", { provider });
}

/** 保存 omp 供应商的用量查询脚本（真源 = OGG meta store） */
export async function updateOmpProviderUsageScript(
  providerId: string,
  usageScript: import("@/types").UsageScript,
): Promise<boolean> {
  return invoke("update_omp_provider_usage_script", {
    id: providerId,
    usageScript,
  });
}

export async function deleteOmpProvider(providerId: string): Promise<void> {
  return invoke("delete_omp_provider", { providerId });
}

/** 「移除」= 仅撤出 live 配置（库条目保留在 meta，可再次「添加」） */
export async function removeOmpProviderFromLive(
  providerId: string,
): Promise<void> {
  return invoke("remove_omp_provider_from_live", { providerId });
}

/** Persist drag-sort order (full visible id list) into the OGG meta store. */
export async function setOmpProvidersOrder(ids: string[]): Promise<void> {
  return invoke("set_omp_providers_order", { ids });
}

// ── Roles（内置 15 个 + 自定义角色键，写 config.yml:modelRoles） ────────────

export async function setOmpRole(roleAssignment: OmpModelRole): Promise<void> {
  return invoke("set_omp_role", { roleAssignment });
}

export async function deleteOmpRole(role: string): Promise<void> {
  return invoke("delete_omp_role", { role });
}

// ── OAuth (drives the installed `omp` CLI's own credential store) ──────────

export interface OmpAuthAccount {
  index: number;
  identity: string;
}

export interface OmpAuthStatus {
  cliAvailable: boolean;
  loggedIn: boolean;
  accounts: OmpAuthAccount[];
  message: string;
}

/** Query real login status + accounts for a provider via `omp token <p> --list`. */
export async function ompAuthStatus(
  providerId: string,
): Promise<OmpAuthStatus> {
  return invoke<OmpAuthStatus>("omp_auth_status", { providerId });
}

/** Launch `omp auth-broker login <p>` in a terminal; OMP runs its native OAuth flow. */
export async function ompAuthLogin(providerId: string): Promise<void> {
  return invoke("omp_auth_login", { providerId });
}

/** Revoke a provider's stored credential. */
export async function ompAuthLogout(providerId: string): Promise<void> {
  return invoke("omp_auth_logout", { providerId });
}

/**
 * List models known to `omp models --json`, filtered by provider.
 *
 * `kind` 透传给 CLI（默认 chat）。非 chat 角色（image / web / speech /
 * dictation / judge）的候选模型属于 image / search / tts / stt / judge 等 kind，
 * 必须显式传 `"all"` 才拿得到，否则 `web`（搜索后端）、`local`（tts/stt）
 * 会返回空列表。
 */
export async function ompListModels(
  providerId: string,
  kind?: string,
): Promise<import("@/types/omp").OmpModelInfo[]> {
  return invoke<import("@/types/omp").OmpModelInfo[]>("omp_list_models", {
    providerId,
    kind: kind ?? null,
  });
}

/**
 * OMP 目录里已启用的供应商（含 models.yml 之外的合成供应商 `web` / `local`
 * 与 OAuth 供应商）。角色选择器用它补齐 OGG 供应商列表里没有的条目。
 */
export async function ompListEnabledProviders(): Promise<
  import("@/types/omp").OmpEnabledProvider[]
> {
  return invoke<import("@/types/omp").OmpEnabledProvider[]>(
    "omp_list_enabled_providers",
  );
}

/**
 * Fetch the model catalog from the provider's upstream `/models` endpoint.
 * `apiKey` may be a plain key, `$ENV_VAR` / `${ENV_VAR}`, or a secret-bridge
 * command (`!cmd` / `$(cmd)`) — resolution happens backend-side.
 * `authHeader=true` sends `Authorization: Bearer`, otherwise `X-Api-Key`.
 */
export async function ompFetchUpstreamModels(
  baseUrl: string,
  apiKey: string,
  authHeader?: boolean,
): Promise<import("@/types/omp").OmpModelInfo[]> {
  return invoke<import("@/types/omp").OmpModelInfo[]>(
    "omp_fetch_upstream_models",
    { baseUrl, apiKey, authHeader: authHeader ?? null },
  );
}
