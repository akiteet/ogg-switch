/**
 * OMP 供应商复制的纯函数部分。
 *
 * OMP 供应商的完整配置（含 id / name）以 JSON 形式内嵌在
 * `Provider.settingsConfig.config` 里，而 OMP 的保存链路只认内嵌配置里的 id：
 * 直接复制会让 Rust 侧按同 id upsert（`insert_preserving_order` 先 retain 再插回原位），
 * 结果是「什么都没复制，却弹了成功提示」。所以复制必须先改写内嵌配置的 id / name。
 */

import type { OmpProviderConfig } from "@/types/omp";

/** 解析 OMP 供应商的内嵌配置（settingsConfig.config 可能是 JSON 串或对象）。 */
export function parseEmbeddedOmpProvider(
  settingsConfig: Record<string, unknown> | undefined,
): OmpProviderConfig | null {
  const raw = settingsConfig?.config;
  if (typeof raw === "string" && raw.trim()) {
    try {
      return JSON.parse(raw) as OmpProviderConfig;
    } catch {
      return null;
    }
  }
  if (raw && typeof raw === "object") {
    return raw as OmpProviderConfig;
  }
  return null;
}

/**
 * 构造副本：内嵌配置换上新 id / 新显示名，其余字段（baseUrl、apiKey、headers、
 * authHeader、models、usageScript、icon…）原样保留。
 *
 * 同时返回改写后的 provider 对象（库写入用）与 settingsConfig（走通用 Provider
 * 载荷时需要）。解析失败返回 null —— 调用方应报错中止，而不是继续走保存
 * （那会变成又一次「同 id upsert 的假成功」）。
 */
export function buildOmpProviderCopy(
  settingsConfig: Record<string, unknown> | undefined,
  copyId: string,
  copyName: string,
): { provider: OmpProviderConfig; settingsConfig: Record<string, unknown> } | null {
  const parsed = parseEmbeddedOmpProvider(settingsConfig);
  if (!parsed || !copyId) return null;
  const provider: OmpProviderConfig = { ...parsed, id: copyId, name: copyName };
  return {
    provider,
    settingsConfig: { ...settingsConfig, config: JSON.stringify(provider) },
  };
}
