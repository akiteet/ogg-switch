import { describe, expect, it } from "vitest";
import type { OmpProviderConfig, OmpSwitchConfig, OmpModelRole } from "@/types/omp";
import {
  OMP_CHAT_ROLES,
  OMP_KIND_ROLES,
  OMP_ROLES,
  validateOmpConfig,
} from "@/utils/ompConfig";

const apiKeyProvider: OmpProviderConfig = {
  id: "workbuddy",
  name: "WorkBuddy",
  type: "api-key",
  category: "api",
  models: [],
  baseUrl: "https://relay.example.com/v1",
  apiKey: "wbk_test",
  api: "openai-completions",
};

const configWith = (roles: OmpModelRole[]): OmpSwitchConfig => ({
  version: 1,
  providers: [apiKeyProvider],
  roles,
});

describe("OMP 内置角色表", () => {
  it("与 OMP 18.x 的 MODEL_ROLES 对齐（chat 10 + kind 5）", () => {
    expect([...OMP_ROLES]).toEqual([
      "default",
      "smol",
      "slow",
      "vision",
      "plan",
      "commit",
      "tiny",
      "memory",
      "task",
      "advisor",
      "image",
      "web",
      "speech",
      "dictation",
      "judge",
    ]);
    expect(OMP_CHAT_ROLES).toHaveLength(10);
    expect(OMP_KIND_ROLES).toHaveLength(5);
  });

  it("补齐了 memory 与 5 个 kind 角色（曾全部缺失）", () => {
    expect([...OMP_ROLES]).toEqual(
      expect.arrayContaining([
        "memory",
        "image",
        "web",
        "speech",
        "dictation",
        "judge",
      ]),
    );
  });

  it("不再把已废弃的 designer 当内置角色，且无重复", () => {
    expect([...OMP_ROLES]).not.toContain("designer");
    expect(new Set(OMP_ROLES).size).toBe(OMP_ROLES.length);
    expect([...OMP_CHAT_ROLES, ...OMP_KIND_ROLES]).toEqual([...OMP_ROLES]);
  });
});

describe("validateOmpConfig 的角色校验", () => {
  it("接受内置角色", () => {
    const errors = validateOmpConfig(
      configWith([{ role: "memory", providerId: "workbuddy", modelId: "m" }]),
    );
    expect(errors.filter((e) => e.field.startsWith("roles"))).toEqual([]);
  });

  it("接受自定义角色名（OMP 支持 config.yml 里自定义 modelRoles 键）", () => {
    const errors = validateOmpConfig(
      configWith([{ role: "designer", providerId: "workbuddy", modelId: "m" }]),
    );
    expect(errors.filter((e) => e.field.startsWith("roles"))).toEqual([]);
  });

  it("仍然拒绝空角色名与重复角色", () => {
    const empty = validateOmpConfig(
      configWith([{ role: "", providerId: "workbuddy", modelId: "m" }]),
    );
    expect(empty.some((e) => e.field === "roles[0].role")).toBe(true);

    const duplicated = validateOmpConfig(
      configWith([
        { role: "default", providerId: "workbuddy", modelId: "m" },
        { role: "default", providerId: "workbuddy", modelId: "m2" },
      ]),
    );
    expect(duplicated.some((e) => e.field === "roles[1].role")).toBe(true);
  });
});
