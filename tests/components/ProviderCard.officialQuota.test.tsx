import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderCard } from "@/components/providers/ProviderCard";
import type { Provider } from "@/types";
import { server } from "../msw/server";
import { createTestQueryClient } from "../utils/testQueryClient";

const TAURI_ENDPOINT = "http://tauri.local";

vi.mock("@/components/providers/ProviderActions", () => ({
  ProviderActions: () => null,
}));

vi.mock("@/components/ProviderIcon", () => ({
  ProviderIcon: () => null,
}));

vi.mock("@/components/SubscriptionQuotaFooter", () => ({
  default: () => <div>official-quota-footer</div>,
}));
vi.mock("@/components/CopilotQuotaFooter", () => ({ default: () => null }));
vi.mock("@/components/CodexOauthQuotaFooter", () => ({
  default: () => null,
}));
vi.mock("@/components/XaiOauthQuotaFooter", () => ({ default: () => null }));
vi.mock("@/components/UsageFooter", () => ({
  default: () => <div>script-usage-footer</div>,
}));

/**
 * 官方类供应商的额度 footer **默认挂载**。
 *
 * 回归背景（v1.1.3）：此前 `officialSubscriptionEnabled` 还要求该供应商先手动配置
 * 一个 `templateType === "official_subscription"` 且启用的用量脚本，而仓库里没有任何
 * 预设/种子注入它 —— 于是 Grok Build、Claude 这类官方卡片默认「什么都没有」。
 * 现在规则是「官方类供应商默认显示，显式 enabled=false 才关闭」。
 */
const officialProvider = (meta: Provider["meta"] = {}): Provider => ({
  id: "grokbuild-official",
  name: "Grok Build Official",
  category: "official",
  settingsConfig: { config: "" },
  meta,
});

const aggregatorProvider = (): Provider => ({
  id: "relay",
  name: "Relay",
  category: "aggregator",
  settingsConfig: { config: "" },
  meta: {
    usage_script: {
      enabled: true,
      language: "javascript",
      code: "return []",
      templateType: "general",
    },
  },
});

function renderCard(provider: Provider, appId: "grokbuild" | "claude" = "grokbuild") {
  return render(
    <QueryClientProvider client={createTestQueryClient()}>
      <ProviderCard
        provider={provider}
        appId={appId}
        isCurrent
        isProxyRunning={false}
        onSwitch={vi.fn()}
        onEdit={vi.fn()}
        onDelete={vi.fn()}
        onConfigureUsage={vi.fn()}
        onOpenWebsite={vi.fn()}
        onDuplicate={vi.fn()}
      />
    </QueryClientProvider>,
  );
}

describe("ProviderCard official quota default", () => {
  beforeEach(() => {
    // ProviderCard 自己会为「多套餐」判定查一次用量；这里给个空结果避免 MSW 未处理告警。
    server.use(
      http.post(`${TAURI_ENDPOINT}/queryProviderUsage`, () =>
        HttpResponse.json({ success: true, data: [] }),
      ),
    );
  });

  it("mounts the official quota footer for an official provider without any usage script", () => {
    renderCard(officialProvider());

    expect(screen.getByText("official-quota-footer")).toBeInTheDocument();
  });

  it("honors an explicit opt-out written by the usage dialog", () => {
    renderCard(
      officialProvider({
        usage_script: {
          enabled: false,
          language: "javascript",
          code: "",
          templateType: "official_subscription",
        },
      }),
    );

    expect(screen.queryByText("official-quota-footer")).toBeNull();
  });

  it("keeps third-party providers on the script-driven footer", () => {
    renderCard(aggregatorProvider());

    expect(screen.getByText("script-usage-footer")).toBeInTheDocument();
    expect(screen.queryByText("official-quota-footer")).toBeNull();
  });

  it("mounts the official quota footer for Antigravity official providers", () => {
    // v1.1.3 接入了 agy 的额度查询（Cloud Code v1internal），白名单同步加 antigravity；
    // 在此之前 antigravity 官方卡片既没有额度、也没有任何提示。
    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <ProviderCard
          provider={{ ...officialProvider(), id: "antigravity-official" }}
          appId="antigravity"
          isCurrent
          isProxyRunning={false}
          onSwitch={vi.fn()}
          onEdit={vi.fn()}
          onDelete={vi.fn()}
          onConfigureUsage={vi.fn()}
          onOpenWebsite={vi.fn()}
          onDuplicate={vi.fn()}
        />
      </QueryClientProvider>,
    );

    expect(screen.getByText("official-quota-footer")).toBeInTheDocument();
  });

  it("does not mount official quota for apps the backend cannot query", () => {
    // 白名单以 providerCapabilities.OFFICIAL_SUBSCRIPTION_APPS 为单一真源：
    // 后端没有对应分支的 app 不该挂一个永远查不到的 footer。
    renderCard(
      { ...officialProvider(), id: "some-official" },
      // @ts-expect-error 该 app 故意不在白名单内（后端无分支）
      "opencode",
    );

    expect(screen.queryByText("official-quota-footer")).toBeNull();
  });
});
