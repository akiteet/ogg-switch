import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { EnvWarningBanner } from "@/components/env/EnvWarningBanner";
import type { EnvConflict } from "@/types/env";

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }));
const mocks = vi.hoisted(() => ({
  deleteEnvVars: vi.fn(),
  envVarsInUse: vi.fn(),
}));
vi.mock("@/lib/api/env", () => ({
  deleteEnvVars: mocks.deleteEnvVars,
  envVarsInUse: mocks.envVarsInUse,
}));

const baseProps = {
  onDismiss: vi.fn(),
  onDeleted: vi.fn(),
};

const conflict = (name: string, sourcePath = "HKEY_CURRENT_USER\\Environment"): EnvConflict => ({
  varName: name,
  varValue: "some-value",
  sourceType: "system",
  sourcePath,
});

describe("EnvWarningBanner 受管键保护", () => {
  it("OGG 受管键（GEMINI_API_KEY / GOOGLE_GEMINI_BASE_URL）不渲染——它们是 agy 的认证凭据，不是冲突", () => {
    render(
      <EnvWarningBanner
        {...baseProps}
        conflicts={[
          conflict("GEMINI_API_KEY"),
          conflict("GOOGLE_GEMINI_BASE_URL"),
        ]}
      />,
    );

    // 整条横幅都不出现（清单被过滤为空）
    expect(screen.queryByText(/检测到遗留的环境变量/)).toBeNull();
    expect(screen.queryByText("GEMINI_API_KEY")).toBeNull();
    expect(screen.queryByText("GOOGLE_GEMINI_BASE_URL")).toBeNull();
  });

  it("大小写不敏感过滤（注册表键大小写不保证）", () => {
    render(
      <EnvWarningBanner
        {...baseProps}
        conflicts={[conflict("gemini_api_key")]}
      />,
    );
    expect(screen.queryByText(/检测到遗留的环境变量/)).toBeNull();
  });

  it("真正的遗留变量照常展示，受管键被剔除后计数正确", async () => {
    const user = userEvent.setup();
    render(
      <EnvWarningBanner
        {...baseProps}
        conflicts={[conflict("GEMINI_API_KEY"), conflict("GEMINI_STALE_KEY")]}
      />,
    );

    // 计数只算未过滤条目：受管的 GEMINI_API_KEY 不进清单（i18n 未加载真实文案，按 key 断言）
    expect(
      screen.getByText("env.warning.description", { exact: false }),
    ).toBeInTheDocument();

    // 展开后：受管的被过滤、遗留的显示（变量名渲染在 <label> 的文本节点里）
    await user.click(screen.getByRole("button", { name: /env\.actions\.expand/ }));
    expect(screen.queryByText("GEMINI_API_KEY")).toBeNull();
    expect(
      screen.getByText((_, element) => element?.textContent === "GEMINI_STALE_KEY"),
    ).toBeInTheDocument();
  });

  it("全部是受管键时返回 null（不弹横幅打扰用户）", () => {
    const { container } = render(
      <EnvWarningBanner {...baseProps} conflicts={[conflict("GEMINI_API_KEY")]} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("删除确认弹窗里标出「正在被供应商使用」的变量", async () => {
    const user = userEvent.setup();
    // GEMINI_STALE_KEY 是某供应商条目里存的凭据
    mocks.envVarsInUse.mockResolvedValue({
      GEMINI_STALE_KEY: ["antigravity · Apizh"],
    });

    render(
      <EnvWarningBanner
        {...baseProps}
        conflicts={[conflict("GEMINI_STALE_KEY")]}
      />,
    );

    await user.click(screen.getByRole("button", { name: /env\.actions\.expand/ }));
    // 勾选后点删除 → 打开确认弹窗（此时才查询引用）
    const checkboxes = screen.getAllByRole("checkbox");
    await user.click(checkboxes[checkboxes.length - 1]!);
    await user.click(
      screen.getByRole("button", { name: /env\.actions\.deleteSelected/ }),
    );

    expect(mocks.envVarsInUse).toHaveBeenCalledWith(["GEMINI_STALE_KEY"]);
    // 使用中警告 + 供应商名出现在确认弹窗里（文本横跨多个节点，按"存在即可"断言）
    expect(
      await screen.findAllByText((_, el) =>
        el?.textContent?.includes("正在被以下供应商使用") ?? false,
      ),
    ).not.toHaveLength(0);
    expect(
      screen.getAllByText((_, el) =>
        el?.textContent?.includes("antigravity · Apizh") ?? false,
      ).length,
    ).toBeGreaterThan(0);
    expect(
      screen.getAllByText((_, el) =>
        el?.textContent?.includes("删除后对应供应商的凭据会失效") ?? false,
      ).length,
    ).toBeGreaterThan(0);
  });

  it("未被任何供应商引用的变量在确认弹窗里如实标注", async () => {
    const user = userEvent.setup();
    mocks.envVarsInUse.mockResolvedValue({});

    render(
      <EnvWarningBanner
        {...baseProps}
        conflicts={[conflict("GEMINI_STALE_KEY")]}
      />,
    );

    await user.click(screen.getByRole("button", { name: /env\.actions\.expand/ }));
    const checkboxes = screen.getAllByRole("checkbox");
    await user.click(checkboxes[checkboxes.length - 1]!);
    await user.click(
      screen.getByRole("button", { name: /env\.actions\.deleteSelected/ }),
    );

    expect(
      await screen.findAllByText((_, el) =>
        el?.textContent?.includes("未被 OGG 供应商引用") ?? false,
      ),
    ).not.toHaveLength(0);
  });
});
