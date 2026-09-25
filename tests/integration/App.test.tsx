import { Suspense, type ComponentType } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { http, HttpResponse } from "msw";
import {
  resetProviderState,
  setCurrentProviderId,
  setProviders,
  setSettings,
  setOmpProviders,
  getOmpLibraryWrites,
  getOmpLiveWrites,
} from "../msw/state";
import { emitTauriEvent } from "../msw/tauriMocks";
import { server } from "../msw/server";

const toastSuccessMock = vi.fn();
const toastErrorMock = vi.fn();
const skillsPanelMocks = vi.hoisted(() => ({
  checkUpdates: vi.fn(),
  openDiscovery: vi.fn(),
}));

// OGG 的产品形态是双应用（grokbuild + omp，见 appConfig.APP_IDS）；
// VisibleApps 的 TS 类型虽只有两个键，App.tsx 用 spread 合并 settings.visibleApps，
// 运行时会保留额外键——测试借此让 pi/claude/codex 的保留代码路径可达。
const ALL_VISIBLE_APPS = {
  grokbuild: true,
  omp: true,
  claude: true,
  codex: true,
  pi: true,
} as unknown as Parameters<typeof setSettings>[0]["visibleApps"];

const makeAllAppsVisible = () => setSettings({ visibleApps: ALL_VISIBLE_APPS });

vi.mock("sonner", () => ({
  toast: {
    success: (...args: unknown[]) => toastSuccessMock(...args),
    error: (...args: unknown[]) => toastErrorMock(...args),
  },
}));

// jsdom 里 framer-motion 的退出动画（AnimatePresence mode="wait"，0.15s）完成时机
// 不稳定，切换应用瞬间新旧两个 ProviderList 并存导致 getByTestId 抛「multiple」。
// 测试环境统一直通：AnimatePresence 直接渲染 children，motion.* 退化为普通元素。
vi.mock("framer-motion", async (importOriginal) => {
  const React = await import("react");
  const actual = await importOriginal<Record<string, unknown>>();
  const stripMotionProps = ({
    initial: _i,
    animate: _a,
    exit: _e,
    transition: _t,
    variants: _v,
    whileHover: _h,
    whileTap: _p,
    layout: _l,
    ...html
  }: Record<string, unknown>) => {
    void _i;
    void _a;
    void _e;
    void _t;
    void _v;
    void _h;
    void _p;
    void _l;
    return html as Record<string, unknown>;
  };
  const passthrough = (tag: string) =>
    function MotionPassthrough({ children, ...rest }: any) {
      return React.createElement(tag, stripMotionProps(rest), children);
    };
  // 必须按 tag 缓存：每次访问都返回新组件类型会让 React 无限重挂载直至 OOM
  const cache = new Map<string, (props: any) => unknown>();
  return {
    ...actual,
    AnimatePresence: ({ children }: any) => children,
    motion: new Proxy(
      {},
      {
        get: (_target, tag: string) => {
          if (!cache.has(tag)) {
            cache.set(tag, passthrough(tag));
          }
          return cache.get(tag);
        },
      },
    ),
  };
});

vi.mock("@/components/providers/ProviderList", () => ({
  ProviderList: ({
    providers,
    currentProviderId,
    onSwitch,
    onEdit,
    onDuplicate,
    onConfigureUsage,
    onOpenWebsite,
    onCreate,
    onDelete,
    onRemoveFromConfig,
  }: any) => (
    <div>
      <div data-testid="provider-list">{JSON.stringify(providers)}</div>
      <div data-testid="current-provider">{currentProviderId}</div>
      <button onClick={() => onSwitch(providers[currentProviderId])}>
        switch
      </button>
      <button onClick={() => onEdit(providers[currentProviderId])}>edit</button>
      <button onClick={() => onDuplicate(providers[currentProviderId])}>
        duplicate
      </button>
      <button onClick={() => onConfigureUsage(providers[currentProviderId])}>
        usage
      </button>
      <button onClick={() => onOpenWebsite("https://example.com")}>
        open-website
      </button>
      <button onClick={() => onDelete(Object.values(providers)[0])}>
        delete
      </button>
      <button onClick={() => onRemoveFromConfig?.(Object.values(providers)[0])}>
        remove
      </button>
      <button onClick={() => onCreate?.()}>create</button>
    </div>
  ),
}));

vi.mock("@/components/providers/AddProviderDialog", () => ({
  AddProviderDialog: ({ open, onOpenChange, onSubmit, appId }: any) =>
    open ? (
      <div data-testid="add-provider-dialog">
        <button
          onClick={() =>
            onSubmit({
              name: `New ${appId} Provider`,
              settingsConfig: {},
              category: "custom",
              sortIndex: 99,
            })
          }
        >
          confirm-add
        </button>
        <button onClick={() => onOpenChange(false)}>close-add</button>
      </div>
    ) : null,
}));

vi.mock("@/components/providers/EditProviderDialog", () => ({
  EditProviderDialog: ({ open, provider, onSubmit, onOpenChange }: any) =>
    open ? (
      <div data-testid="edit-provider-dialog">
        <button
          onClick={() =>
            onSubmit({
              provider: {
                ...provider,
                name: `${provider.name}-edited`,
              },
              originalId: provider.id,
            })
          }
        >
          confirm-edit
        </button>
        <button onClick={() => onOpenChange(false)}>close-edit</button>
      </div>
    ) : null,
}));

vi.mock("@/components/UsageScriptModal", () => ({
  default: ({ isOpen, provider, onSave, onClose }: any) =>
    isOpen ? (
      <div data-testid="usage-modal">
        <span data-testid="usage-provider">{provider?.id}</span>
        <button onClick={() => onSave("script-code")}>save-script</button>
        <button onClick={() => onClose()}>close-usage</button>
      </div>
    ) : null,
}));

vi.mock("@/components/ConfirmDialog", () => ({
  ConfirmDialog: ({ isOpen, message, onConfirm, onCancel }: any) =>
    isOpen ? (
      <div data-testid="confirm-dialog">
        <div data-testid="confirm-message">{message}</div>
        <button onClick={() => onConfirm()}>confirm-delete</button>
        <button onClick={() => onCancel()}>cancel-delete</button>
      </div>
    ) : null,
}));

vi.mock("@/components/AppSwitcher", () => ({
  AppSwitcher: ({ activeApp, onSwitch, visibleApps }: any) => (
    <div data-testid="app-switcher">
      <span>{activeApp}</span>
      <span data-testid="visible-apps">{JSON.stringify(visibleApps)}</span>
      <button onClick={() => onSwitch("claude")}>switch-claude</button>
      <button onClick={() => onSwitch("codex")}>switch-codex</button>
      <button onClick={() => onSwitch("pi")}>switch-pi</button>
    </div>
  ),
}));

vi.mock("@/components/skills/UnifiedSkillsPanel", async () => {
  const React = await import("react");
  const MockUnifiedSkillsPanel = React.forwardRef(
    ({ onCheckUpdatesStateChange }: any, ref) => {
      React.useEffect(() => {
        onCheckUpdatesStateChange?.({ isChecking: false, hasSkills: true });
        return () =>
          onCheckUpdatesStateChange?.({
            isChecking: false,
            hasSkills: false,
          });
      }, [onCheckUpdatesStateChange]);
      React.useImperativeHandle(ref, () => ({
        openDiscovery: skillsPanelMocks.openDiscovery,
        openImport: vi.fn(),
        openInstallFromZip: vi.fn(),
        openRestoreFromBackup: vi.fn(),
        checkUpdates: skillsPanelMocks.checkUpdates,
      }));
      return <div data-testid="unified-skills-panel" />;
    },
  );
  MockUnifiedSkillsPanel.displayName = "MockUnifiedSkillsPanel";
  return { default: MockUnifiedSkillsPanel };
});

vi.mock("@/components/UpdateBadge", () => ({
  UpdateBadge: ({ onClick }: any) => (
    <button onClick={onClick}>update-badge</button>
  ),
}));

vi.mock("@/components/mcp/McpPanel", () => ({
  default: ({ open, onOpenChange }: any) =>
    open ? (
      <div data-testid="mcp-panel">
        <button onClick={() => onOpenChange(false)}>close-mcp</button>
      </div>
    ) : (
      <button onClick={() => onOpenChange(true)}>open-mcp</button>
    ),
}));

const renderApp = (AppComponent: ComponentType) => {
  const client = new QueryClient();
  return render(
    <QueryClientProvider client={client}>
      <Suspense fallback={<div data-testid="loading">loading</div>}>
        <AppComponent />
      </Suspense>
    </QueryClientProvider>,
  );
};

describe("App integration with MSW", () => {
  beforeEach(() => {
    resetProviderState();
    toastSuccessMock.mockReset();
    toastErrorMock.mockReset();
    skillsPanelMocks.checkUpdates.mockReset();
    skillsPanelMocks.openDiscovery.mockReset();
    localStorage.removeItem("ogg-switch-last-app");
    localStorage.removeItem("cc-switch-last-view");
    // App 初始应用固定为 grokbuild（APP_IDS 首项）；给它种子一个供应商，
    // 让「启动即有列表」的用例在 OGG 的双应用模型下成立。
    setProviders("grokbuild", {
      "grok-1": {
        id: "grok-1",
        name: "Grok Default",
        settingsConfig: {},
        category: "official",
        sortIndex: 0,
        createdAt: Date.now(),
      },
    });
    setCurrentProviderId("grokbuild", "grok-1");
    makeAllAppsVisible();
  });

  it("covers basic provider flows via real hooks", async () => {
    const { default: App } = await import("@/App");
    renderApp(App);

    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toContain(
        "grok-1",
      ),
    );

    fireEvent.click(screen.getByText("switch-codex"));
    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toContain(
        "codex-1",
      ),
    );

    fireEvent.click(screen.getByText("usage"));
    expect(screen.getByTestId("usage-modal")).toBeInTheDocument();
    fireEvent.click(screen.getByText("save-script"));
    fireEvent.click(screen.getByText("close-usage"));

    fireEvent.click(screen.getByText("create"));
    expect(screen.getByTestId("add-provider-dialog")).toBeInTheDocument();
    fireEvent.click(screen.getByText("confirm-add"));
    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toMatch(
        /New codex Provider/,
      ),
    );

    fireEvent.click(screen.getByText("edit"));
    expect(screen.getByTestId("edit-provider-dialog")).toBeInTheDocument();
    fireEvent.click(screen.getByText("confirm-edit"));
    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toMatch(
        /-edited/,
      ),
    );

    fireEvent.click(screen.getByText("switch"));
    fireEvent.click(screen.getByText("duplicate"));
    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toMatch(/copy/),
    );

    fireEvent.click(screen.getByText("open-website"));

    emitTauriEvent("provider-switched", {
      appType: "codex",
      providerId: "codex-2",
    });

    expect(toastErrorMock).not.toHaveBeenCalled();
    expect(toastSuccessMock).toHaveBeenCalled();
    // 全量并行跑时 worker 竞争 CPU，10s 会偶发超时（单跑 ~4s）
  }, 30_000);

  it("shows toast when auto sync fails in background", async () => {
    const { default: App } = await import("@/App");
    renderApp(App);

    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toContain(
        "grok-1",
      ),
    );

    expect(() => {
      emitTauriEvent("webdav-sync-status-updated", null);
    }).not.toThrow();
    expect(toastErrorMock).not.toHaveBeenCalled();

    emitTauriEvent("webdav-sync-status-updated", {
      source: "auto",
      status: "error",
      error: "network timeout",
    });

    await waitFor(() => {
      expect(toastErrorMock).toHaveBeenCalled();
    });

    toastErrorMock.mockReset();
    expect(() => {
      emitTauriEvent("s3-sync-status-updated", null);
    }).not.toThrow();
    expect(toastErrorMock).not.toHaveBeenCalled();

    emitTauriEvent("s3-sync-status-updated", {
      source: "auto",
      status: "error",
      error: "s3 timeout",
    });

    await waitFor(() => {
      expect(toastErrorMock).toHaveBeenCalled();
    });
  });

  it("warns without blocking when removing Pi's global default provider", async () => {
    setProviders("pi", {
      custom: {
        id: "custom",
        name: "Custom Pi",
        settingsConfig: {
          baseUrl: "https://api.example.com/v1",
          apiKey: "test-key",
          api: "openai-completions",
          models: [{ id: "model-a" }],
        },
        category: "custom",
        sortIndex: 0,
        createdAt: Date.now(),
      },
    });
    server.use(
      http.post("http://tauri.local/get_pi_current_state", () =>
        HttpResponse.json({
          enabledProviderIds: ["custom"],
          defaultProviderId: "custom",
        }),
      ),
    );

    const { default: App } = await import("@/App");
    renderApp(App);

    // 等注入的 visibleApps 经 settings 查询生效，否则点击会被弹回 effect 重置
    await waitFor(() =>
      expect(screen.getByTestId("visible-apps").textContent).toContain("pi"),
    );
    fireEvent.click(screen.getByText("switch-pi"));

    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toContain(
        "Custom Pi",
      ),
    );
    fireEvent.click(screen.getByText("remove"));

    expect(screen.getByTestId("confirm-message")).toHaveTextContent(
      "confirm.piDefaultProviderWarning",
    );
    fireEvent.click(screen.getByText("confirm-delete"));
    await waitFor(() =>
      expect(screen.queryByTestId("confirm-dialog")).not.toBeInTheDocument(),
    );
  });

  it("hosts the Skills check-update action in the App toolbar", async () => {
    localStorage.setItem("cc-switch-last-view", "skills");
    const { default: App } = await import("@/App");
    renderApp(App);

    expect(
      await screen.findByTestId("unified-skills-panel"),
    ).toBeInTheDocument();
    const checkUpdatesButton = await screen.findByRole("button", {
      name: "skills.checkUpdates",
    });
    await waitFor(() => expect(checkUpdatesButton).toBeEnabled());

    fireEvent.click(checkUpdatesButton);
    expect(skillsPanelMocks.checkUpdates).toHaveBeenCalledTimes(1);
  });

  it("routes the Skills discover toolbar action through the panel guard", async () => {
    localStorage.setItem("cc-switch-last-view", "skills");
    const { default: App } = await import("@/App");
    renderApp(App);

    expect(
      await screen.findByTestId("unified-skills-panel"),
    ).toBeInTheDocument();
    fireEvent.click(
      await screen.findByRole("button", {
        name: "skills.discover",
      }),
    );

    expect(skillsPanelMocks.openDiscovery).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId("unified-skills-panel")).toBeInTheDocument();
  });

  it("duplicates an Oh My Pi provider into the library instead of overwriting it", async () => {
    localStorage.setItem("ogg-switch-last-app", "omp");
    setOmpProviders(
      [
        {
          id: "workbuddy",
          name: "WorkBuddy",
          type: "api-key",
          category: "api",
          models: [],
          baseUrl: "http://127.0.0.1:7864/v1",
          apiKey: "wbk_test",
          api: "openai-completions",
          inConfig: true,
        },
      ],
      [{ role: "default", providerId: "workbuddy", modelId: "cn:hy3" }],
    );

    const { default: App } = await import("@/App");
    renderApp(App);

    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toContain(
        "workbuddy",
      ),
    );

    fireEvent.click(screen.getByText("duplicate"));

    // 副本以新 id 出现在列表里（旧实现是「什么都没复制却提示已添加」）
    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toContain(
        "workbuddy-copy",
      ),
    );
    expect(screen.getByTestId("provider-list").textContent).toContain(
      "WorkBuddy copy",
    );

    // 只写库（未添加状态），不写 models.yml
    expect(getOmpLibraryWrites().map((p) => p.id)).toEqual(["workbuddy-copy"]);
    expect(getOmpLibraryWrites()[0]!.name).toBe("WorkBuddy copy");
    expect(getOmpLiveWrites()).toEqual([]);
    expect(toastSuccessMock).toHaveBeenCalledWith(
      expect.stringContaining("已复制到供应商库"),
    );
  });

  it("maps the Oh My Pi usage script from the camelCase wire key onto provider meta", async () => {
    // 回归（v1.1.3）：后端 `OmpProviderConfig` 是 camelCase 线格式（`usageScript`），
    // 前端曾按 snake_case 读 → meta.usage_script 恒空 → 卡片用量区从未渲染、
    // 用量脚本弹窗也永远回填不出已保存的脚本。
    localStorage.setItem("ogg-switch-last-app", "omp");
    setOmpProviders([
      {
        id: "super-nb",
        name: "SUPER NB",
        type: "api-key",
        category: "api",
        models: [],
        baseUrl: "https://api.super-nb.me/v1",
        apiKey: "snb_test",
        api: "openai-completions",
        inConfig: true,
        usageScript: {
          enabled: true,
          language: "javascript",
          code: "return { remaining: 42, unit: 'CNY' }",
          templateType: "general",
        },
      },
    ]);

    const { default: App } = await import("@/App");
    renderApp(App);

    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toContain(
        "super-nb",
      ),
    );

    // ProviderList 的 mock 会把 providers 原样 JSON 化，这里直接断言映射后的 meta：
    // 用量脚本必须以通用 Provider.meta 的 snake_case 键出现，且保留 enabled/脚本内容。
    const dumped = screen.getByTestId("provider-list").textContent ?? "";
    expect(dumped).toContain('"usage_script":{"enabled":true');
    expect(dumped).toContain("return { remaining: 42, unit: 'CNY' }");
    // 本文件首个跑到 `await import("@/App")` 的用例要付整棵 App 的 transform 成本，
    // 这台机器上会吃掉默认 5s 超时（同文件既有用例更慢）；给个明确上限避免假超时。
  }, 20_000);
});
