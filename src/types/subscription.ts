export type CredentialStatus =
  | "valid"
  | "expired"
  | "not_found"
  | "parse_error";

export interface QuotaTier {
  name: string;
  utilization: number; // 0-100
  resetsAt: string | null;
  usedValueUsd?: number | null;
  maxValueUsd?: number | null;
  planLabel?: string | null;
  /**
   * 稳定身份（React key 用），与显示文本解耦。官方订阅 tier 不设（name 即身份）；
   * OMP 配额窗口由 limitId+accountKey 组合而来，name 则优先用人类可读的
   * window.label——避免把 `openai-codex:primary` 这类原始 id 当标题展示。
   */
  key?: string;
}

export interface ExtraUsage {
  isEnabled: boolean;
  monthlyLimit: number | null;
  usedCredits: number | null;
  utilization: number | null;
  currency: string | null;
}

export interface SubscriptionQuota {
  tool: string;
  credentialStatus: CredentialStatus;
  credentialMessage: string | null;
  success: boolean;
  tiers: QuotaTier[];
  extraUsage: ExtraUsage | null;
  error: string | null;
  queriedAt: number | null;
}
