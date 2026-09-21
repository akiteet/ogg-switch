import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import type { ComponentProps } from "react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createTestQueryClient } from "../utils/testQueryClient";
import type { UsageSummary } from "@/types/usage";

const useUsageSummaryByAppMock = vi.hoisted(() => vi.fn());
const getOmpQuotaWindowsMock = vi.hoisted(() => vi.fn());

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
    i18n: {
      resolvedLanguage: "en",
      language: "en",
    },
  }),
}));

vi.mock("framer-motion", () => ({
  motion: {
    div: ({ children, ...props }: { children?: ReactNode }) => (
      <div {...props}>{children}</div>
    ),
  },
}));

vi.mock("@/lib/query/usage", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/query/usage")>(
      "@/lib/query/usage",
    );
  return {
    ...actual,
    useUsageSummaryByApp: (...args: unknown[]) =>
      useUsageSummaryByAppMock(...args),
  };
});

vi.mock("@/lib/api/usage", () => ({
  usageApi: {
    getOmpQuotaWindows: () => getOmpQuotaWindowsMock(),
  },
}));

import { UsageHero } from "@/components/usage/UsageHero";

const emptySummary: UsageSummary = {
  totalRequests: 0,
  totalCost: "0",
  totalInputTokens: 0,
  totalOutputTokens: 0,
  totalCacheCreationTokens: 0,
  totalCacheReadTokens: 0,
  successRate: 0,
  realTotalTokens: 0,
  cacheHitRate: 0,
};

const range = { preset: "7d" as const };

function renderHero(
  props: Partial<ComponentProps<typeof UsageHero>> = {},
  client = createTestQueryClient(),
) {
  const ui = (
    <QueryClientProvider client={client}>
      <UsageHero range={range} refreshIntervalMs={0} {...props} />
    </QueryClientProvider>
  );
  const view = render(ui);
  return { ...view, ui, client };
}

describe("UsageHero", () => {
  beforeEach(() => {
    useUsageSummaryByAppMock.mockReset();
    getOmpQuotaWindowsMock.mockReset();
    getOmpQuotaWindowsMock.mockResolvedValue([]);
  });

  it("does not change hook count when the summary finishes loading", () => {
    useUsageSummaryByAppMock.mockReturnValue({
      data: undefined,
      isLoading: true,
    });
    const { rerender, ui } = renderHero({ appType: "omp" });

    useUsageSummaryByAppMock.mockReturnValue({
      data: [{ appType: "omp", summary: emptySummary }],
      isLoading: false,
    });

    expect(() => rerender(ui)).not.toThrow();
  });

  it("shows OMP quota window labels when there are no local tokens", async () => {
    useUsageSummaryByAppMock.mockReturnValue({
      data: [{ appType: "omp", summary: emptySummary }],
      isLoading: false,
    });
    getOmpQuotaWindowsMock.mockResolvedValue([
      {
        provider: "openai-codex",
        usedFraction: 0.99,
        label: "30 days",
      },
    ]);

    renderHero({ appType: "omp" });

    await waitFor(() => {
      expect(
        screen.getByText("openai-codex (30 days) 99%"),
      ).toBeInTheDocument();
    });
  });

  it("shows the antigravity empty hint without fetching OMP quotas", async () => {
    useUsageSummaryByAppMock.mockReturnValue({
      data: [{ appType: "antigravity", summary: emptySummary }],
      isLoading: false,
    });

    renderHero({ appType: "antigravity" });

    expect(
      screen.getByText(
        "agy 会话用量来自本地会话数据库；还没有记录时这里为空。",
      ),
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(getOmpQuotaWindowsMock).not.toHaveBeenCalled();
    });
  });
});
