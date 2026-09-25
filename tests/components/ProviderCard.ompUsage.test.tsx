import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { ProviderCard } from "@/components/providers/ProviderCard";
import type { Provider } from "@/types";
import { server } from "../msw/server";
import { createTestQueryClient } from "../utils/testQueryClient";

const TAURI_ENDPOINT = "http://tauri.local";

vi.mock("@/components/providers/ProviderActions", () => ({
  ProviderActions: () => <button>configure-usage</button>,
}));

vi.mock("@/components/ProviderIcon", () => ({
  ProviderIcon: () => null,
}));

/**
 * OMP 供应商卡片的用量区渲染。
 *
 * 回归背景（v1.1.3）：后端 `OmpProviderConfig` 带 `#[serde(rename_all = "camelCase")]`，
 * 用量脚本在线格式里的键名是 `usageScript`；`queries.ts` 的 `ompProviderToProvider()`
 * 曾经按 snake_case 读 `provider.usage_script`，于是 `meta.usage_script` 永远为空、
 * `UsageFooter` 在 `usageEnabled=false` 分支静默 `return null` —— OMP 卡片从来没能
 * 显示过用量。本用例锁死「脚本启用 → 卡片真的渲染出查询结果」这条链路。
 */
const ompProviderWithScript = (): Provider => ({
  id: "super-nb",
  name: "SUPER NB",
  category: "aggregator",
  websiteUrl: "https://api.super-nb.me",
  settingsConfig: { config: "{}" },
  meta: {
    ompInConfig: true,
    usage_script: {
      enabled: true,
      language: "javascript",
      code: "return { remaining: 42, unit: 'CNY' }",
      templateType: "general",
    },
  },
});

function renderCard(provider: Provider) {
  return render(
    <QueryClientProvider client={createTestQueryClient()}>
      <ProviderCard
        provider={provider}
        appId="omp"
        isCurrent={false}
        isInConfig={true}
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

describe("OMP provider card usage footer", () => {
  it("renders the queried usage when the provider has an enabled usage script", async () => {
    server.use(
      http.post(`${TAURI_ENDPOINT}/queryProviderUsage`, () =>
        HttpResponse.json({
          success: true,
          data: [{ remaining: 42, unit: "CNY" }],
        }),
      ),
    );

    renderCard(ompProviderWithScript());

    // 数值与单位来自查询结果，与本机语言无关，断言最稳。
    await waitFor(() => {
      expect(screen.getByText("42.00")).toBeInTheDocument();
    });
    expect(screen.getByText("CNY")).toBeInTheDocument();
  });

  it("keeps the usage area hidden when no script is configured", () => {
    const provider = ompProviderWithScript();
    delete provider.meta!.usage_script;

    renderCard(provider);

    // 没有脚本时不查询、不渲染（与其它 app 的非官方供应商行为一致）。
    expect(screen.queryByText("42.00")).toBeNull();
    expect(screen.queryByText("CNY")).toBeNull();
  });
});
