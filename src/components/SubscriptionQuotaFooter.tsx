import React from "react";
import { RefreshCw, AlertCircle, Clock } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AppId } from "@/lib/api";
import { useSubscriptionQuota } from "@/lib/query/subscription";
import type { QuotaTier, SubscriptionQuota } from "@/types/subscription";

interface SubscriptionQuotaFooterProps {
  appId: AppId;
  inline?: boolean;
  isCurrent?: boolean;
  autoQueryInterval?: number;
}

interface SubscriptionQuotaViewProps {
  quota: SubscriptionQuota | undefined;
  loading: boolean;
  refetch: () => void;
  /** 用于 `subscription.expiredHint` 的 {tool} 插值；解耦了 hook 的 appId */
  appIdForExpiredHint: string;
  inline?: boolean;
}

/**
 * 已知 tier 名称的显示映射（官方订阅 + Token Plan 共用）。
 * `current_plan:` 前缀的 tier（套餐名兜底，见后端 `TIER_CURRENT_PLAN_PREFIX`）
 * 在 `tierLabel()` 里单独处理。
 */
export const TIER_I18N_KEYS: Record<string, string> = {
  five_hour: "subscription.fiveHour",
  seven_day: "subscription.sevenDay",
  seven_day_fable: "subscription.sevenDayFable",
  seven_day_opus: "subscription.sevenDayOpus",
  seven_day_sonnet: "subscription.sevenDaySonnet",
  // Codex 免费方案的次要窗口是 30 天（付费方案为 7 天）
  "30_day": "subscription.thirtyDay",
  // Gemini 模型分类
  gemini_pro: "subscription.geminiPro",
  gemini_flash: "subscription.geminiFlash",
  gemini_flash_lite: "subscription.geminiFlashLite",
  // Antigravity 两大模型族（对齐其官方 UI 的分组）
  gemini_family: "subscription.geminiFamily",
  claude_gpt_family: "subscription.claudeGptFamily",
  // Token Plan（five_hour 已在上方官方映射中）
  weekly_limit: "subscription.sevenDay",
  // 火山方舟 Agent Plan / Coding Plan 的月窗口
  monthly: "subscription.monthly",
  // Grok credit 额度的兜底窗口（重置距离可识别时归入 weekly_limit/monthly）
  credits: "subscription.credits",
  // GitHub Copilot
  premium: "subscription.copilotPremium",
};

/** 根据使用百分比返回颜色 class */
export function utilizationColor(utilization: number): string {
  if (utilization >= 90) return "text-red-500 dark:text-red-400";
  if (utilization >= 70) return "text-orange-500 dark:text-orange-400";
  return "text-green-600 dark:text-green-400";
}

/**
 * tier 的显示标签。`current_plan:<套餐名>` 是"套餐名兜底"（无百分比数据，
 * 见后端 `TIER_CURRENT_PLAN_PREFIX`），直接展示套餐名并加「套餐」前缀。
 */
export function tierLabel(tier: QuotaTier, t: (key: string) => string): string {
  if (tier.name.startsWith("current_plan:")) {
    const plan = tier.name.slice("current_plan:".length);
    return `${t("subscription.currentPlan")} ${plan}`;
  }
  return TIER_I18N_KEYS[tier.name] ? t(TIER_I18N_KEYS[tier.name]) : tier.name;
}

/** 计算倒计时的纯时间字符串，如 "2h30m"、"3d12h" */
export function countdownStr(resetsAt: string | null): string | null {
  if (!resetsAt) return null;
  const diffMs = new Date(resetsAt).getTime() - Date.now();
  if (diffMs <= 0) return null;

  const hours = Math.floor(diffMs / (1000 * 60 * 60));
  const minutes = Math.floor((diffMs % (1000 * 60 * 60)) / (1000 * 60));

  if (hours > 24) {
    const days = Math.floor(hours / 24);
    return `${days}d${hours % 24}h`;
  }
  if (hours > 0) return `${hours}h${minutes}m`;
  return `${minutes}m`;
}

/** 格式化重置时间为倒计时文本（带 i18n 模板） */
export function formatResetTime(
  resetsAt: string | null,
  t: (key: string, options?: Record<string, string>) => string,
): string | null {
  const time = countdownStr(resetsAt);
  if (!time) return null;
  return t("subscription.resetsIn", { time });
}

/** 不需要在 inline 模式显示的 tier */
const HIDDEN_INLINE_TIERS = new Set(["seven_day_sonnet"]);

/** 格式化相对时间（与 UsageFooter 一致） */
function formatRelativeTime(
  timestamp: number,
  now: number,
  t: (key: string, options?: { count?: number }) => string,
): string {
  const diff = Math.floor((now - timestamp) / 1000);
  if (diff < 60) return t("usage.justNow");
  if (diff < 3600)
    return t("usage.minutesAgo", { count: Math.floor(diff / 60) });
  if (diff < 86400)
    return t("usage.hoursAgo", { count: Math.floor(diff / 3600) });
  return t("usage.daysAgo", { count: Math.floor(diff / 86400) });
}

/**
 * 凭据/数据缺失时的**可见**说明。
 *
 * 这几条分支以前一律 `return null`：最典型的是 Grok Build —— 本机没有
 * `~/.grok/auth.json` 时后端返回 `credentialStatus="not_found"`，卡片上
 * 「什么都没有」，用户既不知道是「没登录」还是「功能坏了」（2026-09-24 报障）。
 * 现在统一渲染一行灰色说明 + 刷新按钮，文案里点明该跑哪条 CLI 命令。
 */
const QuotaHint: React.FC<{
  message: string;
  hint?: string;
  loading: boolean;
  refetch: () => void;
  inline?: boolean;
}> = ({ message, hint, loading, refetch, inline }) => {
  const { t } = useTranslation();

  const text = (
    <>
      <AlertCircle
        size={inline ? 12 : 14}
        className="text-muted-foreground flex-shrink-0"
      />
      <span className="text-muted-foreground">{message}</span>
      {hint ? (
        <span className="text-muted-foreground/70">{hint}</span>
      ) : null}
    </>
  );
  const refresh = (
    <button
      onClick={() => refetch()}
      disabled={loading}
      className="p-1 rounded hover:bg-muted transition-colors disabled:opacity-50 flex-shrink-0"
      title={t("subscription.refresh")}
    >
      <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
    </button>
  );

  if (inline) {
    return (
      <div className="inline-flex items-center gap-2 text-xs rounded-lg border border-border-default bg-card px-3 py-2 shadow-sm">
        <div className="flex items-center gap-1.5">{text}</div>
        {refresh}
      </div>
    );
  }
  return (
    <div className="mt-3 rounded-xl border border-border-default bg-card px-4 py-3 shadow-sm">
      <div className="flex items-center justify-between gap-2 text-xs">
        <div className="flex items-center gap-2">{text}</div>
        {refresh}
      </div>
    </div>
  );
};

/**
 * 纯展示组件：渲染 SubscriptionQuota 的 5 种状态（not_found / parse_error /
 * expired / API 失败 / 成功），支持 inline / expanded 两种布局。
 *
 * 数据源由调用方 hook 注入，方便不同的额度后端复用同一套渲染逻辑：
 * - `SubscriptionQuotaFooter`（CLI 凭据路径，by appId）
 * - `CodexOauthQuotaFooter`（cc-switch 自管 OAuth 路径，by ChatGPT account）
 */
export const SubscriptionQuotaView: React.FC<SubscriptionQuotaViewProps> = ({
  quota,
  loading,
  refetch,
  appIdForExpiredHint,
  inline = false,
}) => {
  const { t } = useTranslation();

  // 定期更新相对时间显示
  const [now, setNow] = React.useState(Date.now());
  React.useEffect(() => {
    if (!quota?.queriedAt) return;
    const interval = setInterval(() => setNow(Date.now()), 30000);
    return () => clearInterval(interval);
  }, [quota?.queriedAt]);

  // 还没拿到数据（查询未启用/首次加载中）→ 不占位
  if (!quota) return null;

  // 未查到凭据 → 可见说明（不再静默消失：用户需要知道"去登录哪个 CLI"）
  if (quota.credentialStatus === "not_found") {
    return (
      <QuotaHint
        message={t("subscription.credentialMissing")}
        hint={t("subscription.credentialMissingHint", {
          tool: appIdForExpiredHint,
        })}
        loading={loading}
        refetch={refetch}
        inline={inline}
      />
    );
  }

  // 凭据无法解析 → 可见说明（重新登录即可修复）
  if (quota.credentialStatus === "parse_error") {
    return (
      <QuotaHint
        message={t("subscription.credentialParseError")}
        hint={t("subscription.credentialMissingHint", {
          tool: appIdForExpiredHint,
        })}
        loading={loading}
        refetch={refetch}
        inline={inline}
      />
    );
  }

  // 凭据过期
  if (quota.credentialStatus === "expired" && !quota.success) {
    if (inline) {
      return (
        <div className="inline-flex items-center gap-2 text-xs rounded-lg border border-amber-200 dark:border-amber-800 bg-amber-50 dark:bg-amber-900/20 px-3 py-2 shadow-sm">
          <div className="flex items-center gap-1.5 text-amber-600 dark:text-amber-400">
            <AlertCircle size={12} />
            <span>{t("subscription.expired")}</span>
            {/* 给出可行动的指引：光说"过期"用户不知道下一步该干什么 */}
            <span className="text-amber-500/70 dark:text-amber-400/70">
              {t("subscription.credentialMissingHint", {
                tool: appIdForExpiredHint,
              })}
            </span>
          </div>
          <button
            onClick={() => refetch()}
            disabled={loading}
            className="p-1 rounded hover:bg-muted transition-colors disabled:opacity-50 flex-shrink-0"
            title={t("subscription.refresh")}
          >
            <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      );
    }
    return (
      <div className="mt-3 rounded-xl border border-amber-200 dark:border-amber-800 bg-amber-50 dark:bg-amber-900/20 px-4 py-3 shadow-sm">
        <div className="flex items-center justify-between gap-2 text-xs">
          <div className="flex items-center gap-2 text-amber-600 dark:text-amber-400">
            <AlertCircle size={14} />
            <div>
              <span className="font-medium">{t("subscription.expired")}</span>
              <span className="ml-2 text-amber-500/70 dark:text-amber-400/70">
                {t("subscription.expiredHint", { tool: appIdForExpiredHint })}
              </span>
            </div>
          </div>
          <button
            onClick={() => refetch()}
            disabled={loading}
            className="p-1 rounded hover:bg-amber-100 dark:hover:bg-amber-800/30 transition-colors disabled:opacity-50 flex-shrink-0"
            title={t("subscription.refresh")}
          >
            <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      </div>
    );
  }

  // API 调用失败
  if (!quota.success) {
    if (inline) {
      return (
        <div className="inline-flex items-center gap-2 text-xs rounded-lg border border-border-default bg-card px-3 py-2 shadow-sm">
          <div className="flex items-center gap-1.5 text-red-500 dark:text-red-400">
            <AlertCircle size={12} />
            <span>{t("subscription.queryFailed")}</span>
          </div>
          <button
            onClick={() => refetch()}
            disabled={loading}
            className="p-1 rounded hover:bg-muted transition-colors disabled:opacity-50 flex-shrink-0"
            title={t("subscription.refresh")}
          >
            <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      );
    }
    return (
      <div className="mt-3 rounded-xl border border-border-default bg-card px-4 py-3 shadow-sm">
        <div className="flex items-center justify-between gap-2 text-xs">
          <div className="flex items-center gap-2 text-red-500 dark:text-red-400">
            <AlertCircle size={14} />
            <span>{quota.error || t("subscription.queryFailed")}</span>
          </div>
          <button
            onClick={() => refetch()}
            disabled={loading}
            className="p-1 rounded hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors disabled:opacity-50 flex-shrink-0"
            title={t("subscription.refresh")}
          >
            <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      </div>
    );
  }

  // 成功获取数据
  const tiers = (quota.tiers || []).filter(
    (tier) => tier.name in TIER_I18N_KEYS,
  );
  // 凭据有效但上游没给出可显示的窗口 → 也说一句，避免又退回"什么都没有"
  if (tiers.length === 0) {
    return (
      <QuotaHint
        message={t("subscription.noTiers")}
        loading={loading}
        refetch={refetch}
        inline={inline}
      />
    );
  }

  // ── inline 模式：紧凑两行显示 ──
  if (inline) {
    return (
      <div className="flex flex-col items-end gap-1 text-xs whitespace-nowrap flex-shrink-0">
        {/* 第一行：查询时间 + 刷新 */}
        <div className="flex items-center gap-2 justify-end">
          <span className="text-[10px] text-muted-foreground/70 flex items-center gap-1">
            <Clock size={10} />
            {quota.queriedAt
              ? formatRelativeTime(quota.queriedAt, now, t)
              : t("usage.never", { defaultValue: "从未更新" })}
          </span>
          <button
            onClick={(e) => {
              e.stopPropagation();
              refetch();
            }}
            disabled={loading}
            className="p-1 rounded hover:bg-muted transition-colors disabled:opacity-50 flex-shrink-0 text-muted-foreground"
            title={t("subscription.refresh")}
          >
            <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
          </button>
        </div>

        {/* 第二行：各 tier 使用百分比 */}
        <div className="flex items-center gap-2">
          {tiers
            .filter((tier) => !HIDDEN_INLINE_TIERS.has(tier.name))
            .map((tier) => (
              <TierBadge key={tier.key ?? tier.name} tier={tier} t={t} />
            ))}
        </div>
      </div>
    );
  }

  // ── 展开模式：详细信息 ──
  return (
    <div className="mt-3 rounded-xl border border-border-default bg-card px-4 py-3 shadow-sm">
      <div className="flex items-center justify-between mb-2">
        <span className="text-xs text-gray-500 dark:text-gray-400 font-medium">
          {t("subscription.title", { defaultValue: "Subscription Quota" })}
        </span>
        <div className="flex items-center gap-2">
          {quota.queriedAt && (
            <span className="text-[10px] text-muted-foreground/70 flex items-center gap-1">
              <Clock size={10} />
              {formatRelativeTime(quota.queriedAt, now, t)}
            </span>
          )}
          <button
            onClick={() => refetch()}
            disabled={loading}
            className="p-1 rounded hover:bg-muted transition-colors disabled:opacity-50"
            title={t("subscription.refresh")}
          >
            <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      </div>

      <div className="flex flex-col gap-2">
        {tiers.map((tier) => (
          <TierBar key={tier.key ?? tier.name} tier={tier} t={t} />
        ))}
      </div>

      {/* 超额使用 */}
      {quota.extraUsage?.isEnabled && (
        <div className="mt-2 pt-2 border-t border-border-default text-xs text-gray-500 dark:text-gray-400">
          <span className="font-medium">{t("subscription.extraUsage")}: </span>
          <span className="tabular-nums">
            {/* 同样标口径：哪个是已用、哪个是上限 */}
            {t("subscription.used")} {quota.extraUsage.currency === "USD" ? "$" : ""}
            {(quota.extraUsage.usedCredits ?? 0).toFixed(2)}
            {quota.extraUsage.monthlyLimit != null && (
              <>
                {" "}
                / {t("subscription.upperLimit")}{" "}
                {quota.extraUsage.currency === "USD" ? "$" : ""}
                {quota.extraUsage.monthlyLimit.toFixed(2)}
              </>
            )}
          </span>
        </div>
      )}
    </div>
  );
};

/** 已用百分比（后端 `utilization` 一律是"已用 %"）→ 剩余百分比，clamp 到 0–100 */
export function remainingPercent(utilization: number): number {
  return Math.max(0, Math.min(100, 100 - utilization));
}

/** inline 模式下的单个 tier 显示 */
export const TierBadge: React.FC<{
  tier: QuotaTier;
  t: (key: string, options?: Record<string, unknown>) => string;
}> = ({ tier, t }) => {
  const label = tierLabel(tier, t);
  const countdown = countdownStr(tier.resetsAt);
  // 套餐名兜底 tier（current_plan:）没有百分比数据，只显示套餐名本身
  const isPlanOnly = tier.name.startsWith("current_plan:");
  // 上游不报告百分比（Grok 免费计划）：显示「用量未知」，不把 0.0 当真实已用
  const isUsageUnknown = tier.utilizationUnknown === true;

  const hasUsd = tier.usedValueUsd != null && tier.maxValueUsd != null;

  return (
    <div className="flex items-center gap-0.5">
      {!isPlanOnly && !isUsageUnknown && (
        <span className="text-gray-500 dark:text-gray-400">{label}:</span>
      )}
      {!isPlanOnly && !isUsageUnknown && (
        <span
          className={`font-semibold tabular-nums ${utilizationColor(tier.utilization)}`}
        >
          {t("subscription.used")}{" "}
          {t("subscription.utilization", {
            value: Math.round(tier.utilization),
          })}
        </span>
      )}
      {(isPlanOnly || isUsageUnknown) && (
        <span className="font-semibold text-gray-500 dark:text-gray-400">
          {isUsageUnknown ? `${label}: ` : ""}
          {t("subscription.usageUnknown", { defaultValue: "用量未知" })}
        </span>
      )}
      {hasUsd && !isPlanOnly && (
        <span className="text-muted-foreground/60">
          ({t("subscription.used")} ${tier.usedValueUsd!.toFixed(2)}/
          {t("subscription.upperLimit")} ${tier.maxValueUsd!.toFixed(2)})
        </span>
      )}
      {countdown && (
        <span className="text-muted-foreground/60 ml-0.5 flex items-center gap-px">
          <Clock size={10} />
          {countdown}
        </span>
      )}
    </div>
  );
};

/** 展开模式下的单个 tier 进度条 */
const TierBar: React.FC<{
  tier: QuotaTier;
  t: (key: string, options?: Record<string, unknown>) => string;
}> = ({ tier, t }) => {
  const label = tierLabel(tier, t);
  const resetText = formatResetTime(tier.resetsAt, t);
  const isPlanOnly = tier.name.startsWith("current_plan:");
  const isUsageUnknown = tier.utilizationUnknown === true;

  if (isPlanOnly) {
    return (
      <div className="flex items-center justify-between text-xs">
        <span className="font-medium text-gray-500 dark:text-gray-400">
          {label}
        </span>
        <span className="text-[10px] text-muted-foreground/70">
          {t("subscription.noWindowData")}
        </span>
      </div>
    );
  }

  if (isUsageUnknown) {
    // 上游不报告百分比：只展示窗口与重置时间，进度条不渲染（无数据可画）
    return (
      <div className="flex items-center justify-between text-xs">
        <span className="text-gray-500 dark:text-gray-400 min-w-0 font-medium truncate">
          {label}
        </span>
        <span className="flex items-center gap-2 flex-shrink-0 whitespace-nowrap text-muted-foreground">
          {t("subscription.usageUnknown", { defaultValue: "用量未知" })}
          {resetText && (
            <span className="text-[10px] truncate" title={resetText}>
              {resetText}
            </span>
          )}
        </span>
      </div>
    );
  }

  return (
    <div className="flex items-center gap-3 text-xs">
      <span
        className="text-gray-500 dark:text-gray-400 min-w-0 font-medium"
        style={{ width: "25%" }}
      >
        {label}
      </span>

      {/* 进度条 */}
      <div className="flex-1 h-2 bg-gray-100 dark:bg-gray-800 rounded-full overflow-hidden">
        <div
          className={`h-full rounded-full transition-all ${
            tier.utilization >= 90
              ? "bg-red-500"
              : tier.utilization >= 70
                ? "bg-orange-500"
                : "bg-green-500"
          }`}
          style={{ width: `${Math.min(tier.utilization, 100)}%` }}
        />
      </div>

      <div
        className="flex items-center gap-2 flex-shrink-0 whitespace-nowrap"
        style={{ width: "34%" }}
      >
        {/* 展开态把两个口径都摆出来：后端给的是"已用"，剩余由 100-已用 得出 */}
        <span
          className={`font-semibold tabular-nums ${utilizationColor(tier.utilization)}`}
        >
          {t("subscription.used")}{" "}
          {t("subscription.utilization", { value: Math.round(tier.utilization) })}
        </span>
        <span className="text-muted-foreground">
          {t("subscription.remaining")}{" "}
          {t("subscription.utilization", {
            value: Math.round(remainingPercent(tier.utilization)),
          })}
        </span>
        {resetText && (
          <span
            className="text-[10px] text-muted-foreground/70 truncate"
            title={resetText}
          >
            {resetText}
          </span>
        )}
      </div>
    </div>
  );
};

/**
 * appId → 用户要运行的 CLI 名（用于 `subscription.expiredHint` /
 * `subscription.credentialMissingHint` 的 {tool} 插值）。
 * Grok Build 的命令是 `grok`，Antigravity 的是 `agy`；其余 appId 与命令同名。
 */
const CLI_NAME_BY_APP: Partial<Record<AppId, string>> = {
  grokbuild: "grok",
  antigravity: "agy",
};

/**
 * CLI 凭据路径下的薄 wrapper：通过 useSubscriptionQuota(appId) 自取数据
 * 后转发到 SubscriptionQuotaView。对外 props/行为与重构前完全一致。
 */
const SubscriptionQuotaFooter: React.FC<SubscriptionQuotaFooterProps> = ({
  appId,
  inline = false,
  isCurrent = false,
  autoQueryInterval = 5,
}) => {
  const {
    data: quota,
    isFetching: loading,
    refetch,
  } = useSubscriptionQuota(
    appId,
    isCurrent,
    isCurrent && autoQueryInterval > 0,
    autoQueryInterval,
  );

  if (!isCurrent) return null;

  return (
    <SubscriptionQuotaView
      quota={quota}
      loading={loading}
      refetch={refetch}
      // hint 文案里的 {tool} 是用户要运行的 CLI 名，不是 appId：
      // Grok Build 的命令是 `grok`，Antigravity 的是 `agy`
      appIdForExpiredHint={CLI_NAME_BY_APP[appId] ?? appId}
      inline={inline}
    />
  );
};

export default SubscriptionQuotaFooter;
