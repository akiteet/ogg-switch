import { describe, expect, it } from "vitest";
import { FAILOVER_APPS } from "@/components/settings/ProxyTabContent";

describe("ProxyTabContent failover apps", () => {
  it("only exposes applications with local routing support", () => {
    // OGG 只有 Grok Build 具备完整本地路由数据面（PROXY_APP_IDS）
    expect(FAILOVER_APPS.map(({ id }) => id)).toEqual(["grokbuild"]);
  });
});
