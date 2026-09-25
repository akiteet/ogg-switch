import React from "react";
import { useTranslation } from "react-i18next";
import { Clock } from "lucide-react";
import type { Provider } from "@/types";
import type { OmpQuotaWindow } from "@/lib/api/usage";
import { useOmpQuotaWindows } from "@/lib/query/usage";
import {
  tierLabel,
  utilizationColor,
  remainingPercent,
  countdownStr,
  formatResetTime,
} from "@/components/SubscriptionQuotaFooter";
import type { QuotaTier } from "@/types/subscription";

/**
 * OMP OAuth 供应商的卡片额度区。
 *
 * 数据源是 OMP 自己记在 `~/.omp/agent/agent.db:usage_history` 的配额窗口
 * （`get_omp_quota_windows`）——与「设置 → 用量统计」页面同源；此前这份数据只
 * 在看板提示里出现过，卡片 official 分支因为 omp 不在 `get_subscription_quota`
 * 白名单而渲染 null，额度"查得到却不显示"（2026-09-24 报障）。
 *
 * 匹配键是 `meta.ompOauthProviderId`（凭据库 id，如 `openai-codex`），而不是列表
 * id（`openai`）——与 OmpRoleManager 的 providerRefId 同一约定。
 *
 * **Google 家族（google-antigravity / google-gemini-cli）单独聚合**：OMP 记的
 * 窗口按模型族分（limit_id 形如 `google-antigravity:openai:default:3p-weekly`），
 * 原样渲染会出现两条 "Claude & GPT (shared)" + 一条 "Gemini" 的长条，与
 * Antigravity 应用卡（`get_subscription_quota` 的两族聚合）口径不一致。这里聚成
 * 同样的 `gemini_family` / `claude_gpt_family` 两个 tier，i18n 与应用卡逐字一致。
 */
const GOOGLE_FAMILY_TIERS = ["gemini_family", "claude_gpt_family"] as const;
type GoogleFamily = (typeof GOOGLE_FAMILY_TIERS)[number];

/** 按窗口的 limit_id / label 推断所属模型族；推断不出返回 null（走原样展示）。 */
function googleFamilyOf(window: OmpQuotaWindow): GoogleFamily | null {
  const segments = (window.limitId ?? "").toLowerCase().split(":");
  // google-antigravity:<google|anthropic|openai>:<tier>:<cadence>
  if (segments[1] === "google") return "gemini_family";
  if (segments[1] === "anthropic" || segments[1] === "openai") {
    return "claude_gpt_family";
  }
  const label = (window.label ?? "").toLowerCase();
  if (label.includes("claude") || label.includes("gpt")) {
    return "claude_gpt_family";
  }
  if (label.includes("gemini")) return "gemini_family";
  return null;
}

const OmpQuotaFooter: React.FC<{
  provider: Provider;
  inline?: boolean;
}> = ({ provider, inline = true }) => {
  const { t } = useTranslation();
  const { data: windows = [], isFetching } = useOmpQuotaWindows();

  const oauthId = (
    provider.meta?.ompOauthProviderId ?? provider.id
  )
    .trim()
    .toLowerCase();
  // 不过滤 usedFraction：0% 的窗口（如刚重置的 SuperGrok Weekly）同样是有效信息，
  // 过滤会让 xai 这类账号看起来"没有额度"（v1.1.3 引入过的 bug）。
  const mine = windows.filter(
    (window: OmpQuotaWindow) =>
      window.provider.trim().toLowerCase() === oauthId,
  );

  if (mine.length === 0) return null;

  const toTier = (
    window: OmpQuotaWindow,
    index: number,
  ): QuotaTier => ({
    // key（稳定身份）与 name（显示文本）解耦：窗口的 label（如 "30 days" /
    // "Weekly"）才是给人看的，limitId（`openai-codex:primary`、
    // `xai-oauth:credits:1w`）形态混杂，只适合当 key。
    key: [window.limitId || `${window.provider}:${index}`, window.accountKey]
      .filter(Boolean)
      .join(":"),
    name:
      window.label?.trim() ||
      window.limitId ||
      `${window.provider} #${index + 1}`,
    utilization: Math.min(Math.max(window.usedFraction * 100, 0), 100),
    resetsAt:
      window.resetsAt != null
        ? new Date(window.resetsAt < 1e12 ? window.resetsAt * 1000 : window.resetsAt).toISOString()
        : null,
  });

  let tiers: QuotaTier[];
  if (oauthId.startsWith("google")) {
    // Google 家族：聚合成与 Antigravity 应用卡相同的两族口径。
    // 每族取 used_fraction 最大的窗口（最受限），重复的 shared 行随之去重。
    const byFamily = new Map<GoogleFamily, QuotaTier>();
    const leftovers: QuotaTier[] = [];
    mine.forEach((window: OmpQuotaWindow, index: number) => {
      const family = googleFamilyOf(window);
      if (!family) {
        leftovers.push(toTier(window, index));
        return;
      }
      const tier = toTier({ ...window, label: family }, index);
      const existing = byFamily.get(family);
      if (!existing || tier.utilization > existing.utilization) {
        byFamily.set(family, tier);
      }
    });
    tiers = GOOGLE_FAMILY_TIERS.filter((family) => byFamily.has(family)).map(
      (family) => byFamily.get(family)!,
    );
    tiers.push(...leftovers);
  } else {
    tiers = mine.map(toTier);
  }

  if (inline) {
    // 每个 tier 一行（右对齐、紧凑）：google 家族正好两行（Gemini 系列 / 
    // Claude·GPT 系列），与 Antigravity 应用卡口径一致；徽章横排会把多窗口
    // 挤成一条长 strip（2026-09-25 报障）。
    return (
      <div className="flex flex-col items-end gap-0.5 text-xs flex-shrink-0">
        {tiers.map((tier) => {
          const countdown = countdownStr(tier.resetsAt);
          return (
            <div
              key={tier.key ?? tier.name}
              className="flex items-center gap-1.5 whitespace-nowrap max-w-full"
            >
              <span className="text-gray-500 dark:text-gray-400 font-medium min-w-0 truncate">
                {tierLabel(tier, t)}
              </span>
              <span
                className={`font-semibold tabular-nums flex-shrink-0 ${utilizationColor(tier.utilization)}`}
              >
                {t("subscription.used")}{" "}
                {t("subscription.utilization", {
                  value: Math.round(tier.utilization),
                })}
              </span>
              {countdown && (
                <span className="text-muted-foreground/60 flex items-center gap-px flex-shrink-0">
                  <Clock size={10} />
                  {countdown}
                </span>
              )}
            </div>
          );
        })}
        {isFetching && (
          <span className="text-[10px] text-muted-foreground/70">
            {t("usage.refreshing", { defaultValue: "刷新中…" })}
          </span>
        )}
      </div>
    );
  }

  return (
    <div className="mt-3 rounded-xl border border-border-default bg-card px-4 py-3 shadow-sm">
      <div className="flex flex-col gap-2">
        {tiers.map((tier) => {
          const resetText = formatResetTime(tier.resetsAt, t);
          return (
            <div key={tier.key ?? tier.name} className="text-xs">
              <div className="flex items-center justify-between gap-2 mb-1">
                <span className="text-gray-500 dark:text-gray-400 font-medium min-w-0 truncate">
                  {tierLabel(tier, t)}
                </span>
                <span className="whitespace-nowrap flex-shrink-0">
                  <span
                    className={`font-semibold tabular-nums ${utilizationColor(tier.utilization)}`}
                  >
                    {t("subscription.used")}{" "}
                    {t("subscription.utilization", {
                      value: Math.round(tier.utilization),
                    })}
                  </span>
                  <span className="text-muted-foreground">
                    {" · "}
                    {t("subscription.remaining")}{" "}
                    {t("subscription.utilization", {
                      value: Math.round(remainingPercent(tier.utilization)),
                    })}
                  </span>
                  {resetText && (
                    <span
                      className="text-muted-foreground/60 ml-0.5"
                      title={resetText}
                    >
                      {" · "}
                      {resetText}
                    </span>
                  )}
                </span>
              </div>
              <div className="h-2 bg-gray-100 dark:bg-gray-800 rounded-full overflow-hidden">
                <div
                  className={`h-full rounded-full ${
                    tier.utilization >= 90
                      ? "bg-red-500"
                      : tier.utilization >= 70
                        ? "bg-orange-500"
                        : "bg-green-500"
                  }`}
                  style={{ width: `${Math.min(tier.utilization, 100)}%` }}
                />
              </div>
            </div>
          );
        })}
        {isFetching && (
          <span className="text-[10px] text-muted-foreground/70 text-right">
            {t("usage.refreshing", { defaultValue: "刷新中…" })}
          </span>
        )}
      </div>
    </div>
  );
};

export default OmpQuotaFooter;
