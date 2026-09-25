import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { createInstance } from "i18next";
import { I18nextProvider, initReactI18next } from "react-i18next";
import { http, HttpResponse } from "msw";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderCard } from "@/components/providers/ProviderCard";
import type { Provider } from "@/types";
import zh from "@/i18n/locales/zh.json";
import { server } from "../msw/server";
import { createTestQueryClient } from "../utils/testQueryClient";

const TAURI_ENDPOINT = "http://tauri.local";

// TierBadge 的文案走 i18n（"已用 {{value}}%"），测试环境默认资源为空，
// 必须挂真实 zh 资源才能断言具体数字。
const i18n = createInstance();
beforeAll(async () => {
  await i18n.use(initReactI18next).init({
    lng: "zh",
    resources: { zh: { translation: zh } },
    interpolation: { escapeValue: false },
  });
});

vi.mock("@/components/providers/ProviderActions", () => ({
  ProviderActions: () => null,
}));

vi.mock("@/components/ProviderIcon", () => ({
  ProviderIcon: () => null,
}));

vi.mock("@/components/SubscriptionQuotaFooter", async (importOriginal) => ({
  ...(await importOriginal<
    typeof import("@/components/SubscriptionQuotaFooter")
  >()),
  // 只把默认导出（SubscriptionQuotaFooter 组件）mock 掉：OmpQuotaFooter 复用
  // 该模块的 TierBadge / utilizationColor 等展示件，必须保留真实实现。
  default: () => null,
}));
vi.mock("@/components/CopilotQuotaFooter", () => ({ default: () => null }));
vi.mock("@/components/CodexOauthQuotaFooter", () => ({ default: () => null }));
vi.mock("@/components/XaiOauthQuotaFooter", () => ({ default: () => null }));
vi.mock("@/components/UsageFooter", () => ({
  default: () => <div>script-usage-footer</div>,
}));

/**
 * OMP OAuth 供应商卡片显示本地配额窗口。
 *
 * 回归背景（v1.1.3）：`get_omp_quota_windows` 的数据（与用量看板同源）从未接到卡片
 * —— OMP OAuth 条目 category=subscription→official，而卡片 official 分支只挂
 * `SubscriptionQuotaFooter`，其白名单不含 omp ⇒ 渲染 null，额度"查得到却不显示"。
 *
 * 测试环境的 i18n 资源为空（`t()` 原样返回 key），TierBadge 渲染的是
 * `已用 {value}%` 形态的 key 拼接，所以断言 tier 文本与百分比。
 */
const ompOAuthCard = (): Provider => ({
  id: "openai",
  name: "OpenAI",
  category: "official",
  settingsConfig: {
    config: JSON.stringify({
      id: "openai",
      type: "oauth",
      category: "subscription",
      oauthProviderId: "openai-codex",
    }),
  },
  meta: { ompInConfig: true, ompOauthProviderId: "openai-codex" },
});

const quotaWindows = [
  {
    provider: "openai-codex",
    usedFraction: 0.99,
    label: "30 days",
    resetsAt: 1789826802000,
    accountKey: "oauth|account:u1",
    limitId: "openai-codex:primary",
  },
];

function renderCard(provider: Provider) {
  return render(
    <I18nextProvider i18n={i18n}>
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
    </I18nextProvider>,
  );
}

describe("OMP OAuth provider card quota windows", () => {
  beforeEach(() => {
    server.use(
      http.post(`${TAURI_ENDPOINT}/queryProviderUsage`, () =>
        HttpResponse.json({ success: true, data: [] }),
      ),
    );
  });

  it("renders the quota window matched via ompOauthProviderId", async () => {
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_omp_quota_windows`, () =>
        HttpResponse.json(quotaWindows),
      ),
    );

    renderCard(ompOAuthCard());

    // usedFraction 0.99 → 已用 99%（TierBadge 的百分比文本）
    await screen.findByText("已用 99%");
    // 窗口标题显示人类可读的 label，limitId（`openai-codex:primary`）只作
    // React key，不再原文暴露（2026-09-24 窗口显示统一）
    expect(screen.getByText(/30 days/)).toBeInTheDocument();
    expect(screen.queryByText(/openai-codex:primary/)).toBeNull();
  });

  it("falls back to the raw limitId when a window has no label", async () => {
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_omp_quota_windows`, () =>
        HttpResponse.json([
          {
            provider: "openai-codex",
            usedFraction: 0.4,
            resetsAt: 1789826802000,
            limitId: "openai-codex:primary",
          },
        ]),
      ),
    );

    renderCard(ompOAuthCard());

    // 无 label 的窗口回落 limitId 展示（总比没有标题强）
    await screen.findByText("已用 40%");
    expect(screen.getByText(/openai-codex:primary/)).toBeInTheDocument();
  });

  it("aggregates google-family windows into the same two families as the antigravity card", async () => {
    // 真实 agent.db 形态（2026-09-25）：google-antigravity 记 3 行——
    // openai / anthropic 的 3p-weekly 各一条（shared 族）+ gemini-weekly。
    // 卡片必须聚合成与 Antigravity 应用卡一致的两族（去重 shared、族名共用
    // subscription.geminiFamily / .claudeGptFamily），而不是三条原始窗口。
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_omp_quota_windows`, () =>
        HttpResponse.json([
          {
            provider: "google-antigravity",
            usedFraction: 0.0,
            label: "Claude & GPT (shared)",
            resetsAt: 1790400000000,
            accountKey: "oauth|email:a@gmail.com|project:p",
            limitId: "google-antigravity:openai:default:3p-weekly",
          },
          {
            provider: "google-antigravity",
            usedFraction: 0.0,
            label: "Claude & GPT (shared)",
            resetsAt: 1790400000000,
            accountKey: "oauth|email:a@gmail.com|project:p",
            limitId: "google-antigravity:anthropic:default:3p-weekly",
          },
          {
            provider: "google-antigravity",
            usedFraction: 0.1375,
            label: "Gemini",
            resetsAt: 1789795200000,
            accountKey: "oauth|email:a@gmail.com|project:p",
            limitId: "google-antigravity:google:default:gemini-weekly",
          },
        ]),
      ),
    );

    const provider = ompOAuthCard();
    provider.meta = {
      ...provider.meta,
      ompOauthProviderId: "google-antigravity",
    };
    renderCard(provider);

    await screen.findByText("已用 14%");
    expect(screen.getByText(/Gemini 系列/)).toBeInTheDocument();
    expect(screen.getByText(/Claude \/ GPT 系列/)).toBeInTheDocument();
    // shared 的两行去重成一条；原始窗口名不再出现
    expect(screen.queryByText(/Claude & GPT \(shared\)/)).toBeNull();
    expect(screen.queryByText(/google-antigravity:/)).toBeNull();
  });

  it("matches no window when the provider ids differ", async () => {
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_omp_quota_windows`, () =>
        HttpResponse.json([
          { provider: "some-other-provider", usedFraction: 0.5, label: "Weekly" },
        ]),
      ),
    );

    renderCard(ompOAuthCard());

    // 没有匹配窗口 → 不渲染额度区（也不渲染脚本 footer：official 分支）
    expect(screen.queryByText(/已用 50%/)).toBeNull();
    expect(screen.queryByText("script-usage-footer")).toBeNull();
  });

  it("shows a fully unused window (usedFraction 0) instead of hiding it", async () => {
    // v1.1.3 回归：曾用 usedFraction > 0 过滤，把 xai 这种 0% 的窗口藏掉，
    // 看起来像"只支持 openai"。0% 也是有效信息，必须显示。
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_omp_quota_windows`, () =>
        HttpResponse.json([
          {
            provider: "openai-codex",
            usedFraction: 0,
            label: "Weekly",
            resetsAt: 1789826802000,
            limitId: "openai-codex:primary",
          },
        ]),
      ),
    );

    renderCard(ompOAuthCard());

    // inline 的 TierBadge 只有「已用 0%」；剩余口径只在展开态（TierBar）出现
    await screen.findByText("已用 0%");
  });

  it("does not render the quota area when there are no windows at all", async () => {
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_omp_quota_windows`, () =>
        HttpResponse.json([]),
      ),
    );

    renderCard(ompOAuthCard());

    expect(screen.queryByText(/openai-codex/)).toBeNull();
  });
});
