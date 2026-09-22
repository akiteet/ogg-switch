import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { OmpRoleManager } from "@/components/providers/OmpRoleManager";
import type { OmpModelRole, OmpProviderConfig } from "@/types/omp";

// radix Select 打开时会调用 scrollIntoView（jsdom 未实现）
Element.prototype.scrollIntoView = vi.fn();

const mocks = vi.hoisted(() => ({
  ompListEnabledProviders: vi.fn(),
  ompListModels: vi.fn(),
  setOmpRole: vi.fn().mockResolvedValue(undefined),
  deleteOmpRole: vi.fn().mockResolvedValue(undefined),
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));

vi.mock("sonner", () => ({ toast: mocks.toast }));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    ompApi: {
      ...actual.ompApi,
      ompListEnabledProviders: mocks.ompListEnabledProviders,
      ompListModels: mocks.ompListModels,
      setOmpRole: mocks.setOmpRole,
      deleteOmpRole: mocks.deleteOmpRole,
    },
  };
});

/** 本机真实形态：models.yml 里的中转站 + 与 OMP TUI 一致的 10 条角色（含自定义 designer） */
const providers: OmpProviderConfig[] = [
  {
    id: "workbuddy",
    name: "WorkBuddy",
    type: "api-key",
    category: "api",
    models: [
      { id: "cn:deepseek-v4.1-flash", name: "Deepseek V4.1 Flash", contextWindow: 977000, maxTokens: 125000 },
    ],
    baseUrl: "http://127.0.0.1:7864/v1",
    apiKey: "wbk_test",
    api: "openai-completions",
  },
];

const roles: OmpModelRole[] = [
  { role: "advisor", providerId: "openai-codex", modelId: "gpt-5.6-terra", thinkingLevel: "high" },
  { role: "plan", providerId: "Rigel", modelId: "grok-4.6", thinkingLevel: "high" },
  { role: "default", providerId: "workbuddy", modelId: "cn:deepseek-v4.1-flash" },
  { role: "tiny", providerId: "workbuddy", modelId: "cn:hy3", thinkingLevel: "low" },
  { role: "task", providerId: "workbuddy", modelId: "cn:deepseek-v4.1-flash" },
  { role: "designer", providerId: "workbuddy", modelId: "cn:kimi-k2.8-preview", thinkingLevel: "high" },
  { role: "vision", providerId: "workbuddy", modelId: "cn:deepseek-v4.1-flash" },
  { role: "commit", providerId: "workbuddy", modelId: "cn:deepseek-v4.1-flash" },
  { role: "slow", providerId: "workbuddy", modelId: "cn:glm-5.3-flash", thinkingLevel: "high" },
  { role: "smol", providerId: "workbuddy", modelId: "cn:deepseek-v4.1-flash", thinkingLevel: "high" },
];

const rowFor = (label: string) => screen.getByText(label).closest("tr") as HTMLElement;

beforeEach(() => {
  mocks.ompListEnabledProviders.mockResolvedValue([]);
  mocks.ompListModels.mockResolvedValue([]);
});

describe("OmpRoleManager 角色列表", () => {
  it("列出全部内置角色（含 memory 与 kind 组）与自定义角色，计数与 OMP 一致", async () => {
    render(
      <OmpRoleManager providers={providers} roles={roles} onRolesChange={() => {}} />,
    );

    // 等 OMP 目录查询落地，避免 effect 在断言后才 setState
    await waitFor(() => expect(mocks.ompListEnabledProviders).toHaveBeenCalled());

    // 已配置 10 / 已知 16（内置 15 + 自定义 designer 1），与 OMP TUI 的 10/16 对齐
    expect(screen.getByText(/10\s*\/\s*16/)).toBeInTheDocument();

    // 曾经完全缺失的角色补上了
    expect(rowFor("Memory")).toBeInTheDocument();
    expect(rowFor("Tiny")).toBeInTheDocument();
    expect(rowFor("Judge")).toBeInTheDocument();
    expect(rowFor("Dictation")).toBeInTheDocument();

    // 自定义角色（OMP 已废弃的内置名，现在只是 config.yml 里的自定义键）照常可见可改
    expect(rowFor("designer")).toBeInTheDocument();

    // 分组标题
    expect(screen.getByText("对话角色")).toBeInTheDocument();
    expect(screen.getByText("种类角色（kind）")).toBeInTheDocument();
    expect(screen.getByText("自定义角色")).toBeInTheDocument();
  });

  it("kind 角色可选到 OMP 合成供应商（web / local）并按 kind 过滤候选模型", async () => {
    const user = userEvent.setup();
    mocks.ompListEnabledProviders.mockResolvedValue([
      { id: "web", modelCount: 20 },
      { id: "local", modelCount: 13 },
    ]);
    mocks.ompListModels.mockResolvedValue([
      { id: "parallel", name: "parallel", kind: "search", contextWindow: 0, maxTokens: 0 },
      { id: "brave", name: "brave", kind: "search", contextWindow: 0, maxTokens: 0 },
      { id: "kokoro", name: "kokoro", kind: "tts", contextWindow: 0, maxTokens: 0 },
    ]);

    render(
      <OmpRoleManager providers={providers} roles={roles} onRolesChange={() => {}} />,
    );

    // 打开 Web 角色的编辑弹窗（未分配 → 只有一个 + 按钮）
    const webRow = rowFor("Web");
    await user.click(within(webRow).getAllByRole("button")[0]!);

    const dialog = await screen.findByRole("dialog");
    const comboboxes = within(dialog).getAllByRole("combobox");

    // Provider 下拉里能选到 OMP 内置的 web（不在 models.yml / OGG 供应商列表里）
    await user.click(comboboxes[0]!);
    await user.click(
      await screen.findByRole("option", { name: /web · OMP 内置/ }),
    );

    await waitFor(() => expect(mocks.ompListModels).toHaveBeenCalledWith("web", "all"));

    // 选定供应商后才出现模型下拉（候选来自上面那次目录查询）
    await waitFor(() =>
      expect(within(screen.getByRole("dialog")).getAllByRole("combobox")).toHaveLength(2),
    );
    const modelCombobox = within(screen.getByRole("dialog")).getAllByRole("combobox")[1]!;
    await user.click(modelCombobox);

    // 模型候选按角色接受的 kind 过滤：search 可选，tts 不可选
    expect(await screen.findByRole("option", { name: /parallel/ })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: /kokoro/ })).toBeNull();
  });

  it("Image 角色不列出自建 chat 模型（OMP 不会采纳），改为给出原因与手动输入兜底", async () => {
    const user = userEvent.setup();
    // Rigel 配了 3 个模型（models.yml 条目没有 kind，OMP 一律当 chat）；
    // 目录里也没有 kind=image 的条目——OMP 的 image 候选池就是空的
    const rigel: OmpProviderConfig[] = [
      {
        id: "Rigel",
        name: "Rigel",
        type: "api-key",
        category: "api",
        models: [
          { id: "grok-4.5", name: "grok-4.5", contextWindow: 500000, maxTokens: 64000 },
          { id: "grok-4.6", name: "grok-4.6", contextWindow: 500000, maxTokens: 64000 },
          { id: "grok-imagine-image-2.0", name: "Grok Imagine Image 2.0", contextWindow: 0, maxTokens: 0 },
        ],
        baseUrl: "https://sub.flyli.cn/v1",
        apiKey: "sk-test",
        api: "openai-completions",
      },
    ];
    mocks.ompListEnabledProviders.mockResolvedValue([]);
    mocks.ompListModels.mockResolvedValue([]);

    render(
      <OmpRoleManager providers={rigel} roles={roles} onRolesChange={() => {}} />,
    );

    const imageRow = rowFor("Image");
    await user.click(within(imageRow).getAllByRole("button")[0]!);

    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getAllByRole("combobox")[0]!);
    await user.click(await screen.findByRole("option", { name: /Rigel/ }));

    // 候选为空 → 只有 Provider 一个下拉 + 手动输入兜底（目录查询结束后）
    expect(
      await screen.findByPlaceholderText(/手动输入模型 ID/),
    ).toBeInTheDocument();
    expect(within(screen.getByRole("dialog")).getAllByRole("combobox")).toHaveLength(1);
    // 说明为什么自己的模型不在列表里（而不是让人以为界面不认配置）
    expect(screen.getByText(/kind = image/)).toBeInTheDocument();
    expect(screen.getByText(/3 个自建模型/)).toBeInTheDocument();
  });

  it("Image 角色列出目录里 kind=image 的模型", async () => {
    const user = userEvent.setup();
    mocks.ompListEnabledProviders.mockResolvedValue([
      { id: "openai-codex", modelCount: 4 },
    ]);
    mocks.ompListModels.mockResolvedValue([
      { id: "gpt-image-1", name: "gpt-image-1", kind: "image", contextWindow: 0, maxTokens: 0 },
      { id: "gpt-5.6-terra", name: "gpt-5.6-terra", kind: "chat", contextWindow: 0, maxTokens: 0 },
    ]);

    render(
      <OmpRoleManager providers={providers} roles={roles} onRolesChange={() => {}} />,
    );

    const imageRow = rowFor("Image");
    await user.click(within(imageRow).getAllByRole("button")[0]!);

    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getAllByRole("combobox")[0]!);
    await user.click(
      await screen.findByRole("option", { name: /openai-codex · OMP 内置/ }),
    );

    await waitFor(() =>
      expect(within(screen.getByRole("dialog")).getAllByRole("combobox")).toHaveLength(2),
    );
    await user.click(within(screen.getByRole("dialog")).getAllByRole("combobox")[1]!);
    expect(await screen.findByRole("option", { name: /gpt-image-1/ })).toBeInTheDocument();
    // chat 模型不属于 image 候选池
    expect(screen.queryByRole("option", { name: /gpt-5.6-terra/ })).toBeNull();
  });

  it("chat 角色（Memory）仍列出该供应商已配置的模型", async () => {
    const user = userEvent.setup();
    render(
      <OmpRoleManager providers={providers} roles={roles} onRolesChange={() => {}} />,
    );

    const memoryRow = rowFor("Memory");
    await user.click(within(memoryRow).getAllByRole("button")[0]!);

    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getAllByRole("combobox")[0]!);
    await user.click(await screen.findByRole("option", { name: /WorkBuddy/ }));

    await waitFor(() =>
      expect(within(screen.getByRole("dialog")).getAllByRole("combobox")).toHaveLength(2),
    );
    await user.click(within(screen.getByRole("dialog")).getAllByRole("combobox")[1]!);
    expect(
      await screen.findByRole("option", { name: /Deepseek V4.1 Flash/ }),
    ).toBeInTheDocument();
    expect(screen.queryByPlaceholderText(/手动输入模型 ID/)).toBeNull();
  });

  it("Web 角色只列 search 后端，不再列自建 chat 模型", async () => {
    const user = userEvent.setup();
    // workbuddy 是自建 chat 供应商；目录里只有 web 供应商（真·search 后端）有候选
    // omp_list_models 是按 provider 过滤的，mock 必须照此实现
    mocks.ompListEnabledProviders.mockResolvedValue([{ id: "web", modelCount: 20 }]);
    mocks.ompListModels.mockImplementation((id: string) =>
      Promise.resolve(
        id === "web"
          ? [{ id: "parallel", name: "parallel", kind: "search", contextWindow: 0, maxTokens: 0 }]
          : [],
      ),
    );

    render(
      <OmpRoleManager providers={providers} roles={roles} onRolesChange={() => {}} />,
    );

    const webRow = rowFor("Web");
    await user.click(within(webRow).getAllByRole("button")[0]!);
    const dialog = await screen.findByRole("dialog");

    // 先选自建 chat 供应商 → 候选为空 + 原因说明
    await user.click(within(dialog).getAllByRole("combobox")[0]!);
    await user.click(await screen.findByRole("option", { name: /WorkBuddy/ }));
    expect(
      await screen.findByPlaceholderText(/手动输入模型 ID/),
    ).toBeInTheDocument();
    expect(screen.getByText(/kind = search/)).toBeInTheDocument();

    // 改选 OMP 内置的 web → 只出现 search 后端
    await user.click(within(screen.getByRole("dialog")).getAllByRole("combobox")[0]!);
    await user.click(await screen.findByRole("option", { name: /web · OMP 内置/ }));
    await waitFor(() =>
      expect(within(screen.getByRole("dialog")).getAllByRole("combobox")).toHaveLength(2),
    );
    await user.click(within(screen.getByRole("dialog")).getAllByRole("combobox")[1]!);
    expect(await screen.findByRole("option", { name: /parallel/ })).toBeInTheDocument();
  });

  it("新建自定义角色：输入任意名称保存后写入该角色", async () => {
    const user = userEvent.setup();
    render(
      <OmpRoleManager providers={providers} roles={[]} onRolesChange={() => {}} />,
    );

    await user.click(
      screen.getByRole("button", { name: /添加自定义角色/ }),
    );

    const dialog = await screen.findByRole("dialog");
    const nameInput = within(dialog).getByPlaceholderText(/例如 reviewer/);
    await user.type(nameInput, "reviewer");

    // 选供应商和模型
    const comboboxes = within(dialog).getAllByRole("combobox");
    await user.click(comboboxes[0]!);
    await user.click(await screen.findByRole("option", { name: /WorkBuddy/ }));
    await waitFor(() =>
      expect(within(dialog).getAllByRole("combobox")).toHaveLength(2),
    );
    await user.click(within(dialog).getAllByRole("combobox")[1]!);
    await user.click(
      await screen.findByRole("option", { name: /Deepseek V4.1 Flash/ }),
    );

    await user.click(within(dialog).getByRole("button", { name: "保存" }));

    await waitFor(() => expect(mocks.setOmpRole).toHaveBeenCalledTimes(1));
    expect(mocks.setOmpRole.mock.calls[0]![0]).toMatchObject({
      role: "reviewer",
      providerId: "workbuddy",
      modelId: "cn:deepseek-v4.1-flash",
    });
  });

  it("自定义角色名称校验：重名 / 含空格时禁止保存", async () => {
    const user = userEvent.setup();
    render(
      <OmpRoleManager
        providers={providers}
        roles={[{ role: "designer", providerId: "workbuddy", modelId: "m" }]}
        onRolesChange={() => {}}
      />,
    );

    await user.click(screen.getByRole("button", { name: /添加自定义角色/ }));
    const dialog = await screen.findByRole("dialog");
    const nameInput = within(dialog).getByPlaceholderText(/例如 reviewer/);

    await user.type(nameInput, "designer");
    expect(await within(dialog).findByText("该角色已存在")).toBeInTheDocument();
    expect(
      (within(dialog).getByRole("button", { name: "保存" }) as HTMLButtonElement).disabled,
    ).toBe(true);

    await user.clear(nameInput);
    await user.type(nameInput, "two words");
    expect(
      await within(dialog).findByText("角色名称不能包含空格"),
    ).toBeInTheDocument();
  });
});
