import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SubscriptionQuotaView } from "@/components/SubscriptionQuotaFooter";
import type { SubscriptionQuota } from "@/types/subscription";

/**
 * 凭据缺失/无窗口时**必须可见**。
 *
 * 回归背景（v1.1.3）：`not_found` / `parse_error` / tiers 为空三条分支以前一律
 * `return null`，于是「本机没登录 CLI」与「功能坏了」在界面上完全同形——Grok Build
 * 官方卡片正是这样长期空白且无从排查。
 *
 * 测试环境的 i18n 资源为空，`t()` 会原样返回 key，所以断言 key 而不是中文文案。
 */
const quotaWith = (patch: Partial<SubscriptionQuota>): SubscriptionQuota => ({
  tool: "grokbuild",
  credentialStatus: "valid",
  credentialMessage: null,
  success: true,
  tiers: [],
  extraUsage: null,
  error: null,
  queriedAt: null,
  ...patch,
});

const renderView = (quota: SubscriptionQuota | undefined) =>
  render(
    <SubscriptionQuotaView
      quota={quota}
      loading={false}
      refetch={vi.fn()}
      appIdForExpiredHint="grok"
      inline
    />,
  );

describe("SubscriptionQuotaView missing-credential hints", () => {
  it("shows a hint (with the CLI to log in) when no credentials were found", () => {
    renderView(quotaWith({ credentialStatus: "not_found", success: false }));

    expect(
      screen.getByText("subscription.credentialMissing"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("subscription.credentialMissingHint"),
    ).toBeInTheDocument();
  });

  it("shows a hint when the stored credentials cannot be parsed", () => {
    renderView(quotaWith({ credentialStatus: "parse_error", success: false }));

    expect(
      screen.getByText("subscription.credentialParseError"),
    ).toBeInTheDocument();
  });

  it("shows a hint when credentials are valid but no quota window came back", () => {
    renderView(quotaWith({ tiers: [] }));

    expect(screen.getByText("subscription.noTiers")).toBeInTheDocument();
  });

  it("stays out of the way while the first query has not resolved yet", () => {
    // 查询未启用 / 首次加载中：`quota` 为 undefined，此时不该占位（否则卡片会闪提示）
    const { container } = renderView(undefined);

    expect(container).toBeEmptyDOMElement();
  });

  it("still renders the concrete failure state for API errors", () => {
    // inline 只给一行「查询失败」+ 刷新；展开态才带具体 error 文本
    const inline = renderView(quotaWith({ success: false, error: "HTTP 500" }));
    expect(screen.getByText("subscription.queryFailed")).toBeInTheDocument();
    inline.unmount();

    render(
      <SubscriptionQuotaView
        quota={quotaWith({ success: false, error: "HTTP 500" })}
        loading={false}
        refetch={vi.fn()}
        appIdForExpiredHint="grok"
      />,
    );
    expect(screen.getByText("HTTP 500")).toBeInTheDocument();
  });
});
