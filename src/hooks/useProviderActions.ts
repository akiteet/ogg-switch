import { useCallback } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import {
  ompApi,
  piApi,
  providersApi,
  settingsApi,
  type AppId,
} from "@/lib/api";
import type { Provider, UsageScript } from "@/types";
import type { OmpProviderConfig } from "@/types/omp";
import { injectCodingPlanUsageScript } from "@/config/codingPlanProviders";
import {
  useAddProviderMutation,
  useUpdateProviderMutation,
  useDeleteProviderMutation,
  useSwitchProviderMutation,
} from "@/lib/query";
import { usageKeys } from "@/lib/query/usage";
import { extractErrorMessage } from "@/utils/errorUtils";
import {
  extractCodexWireApi,
  isCodexAnthropicWireApi,
  isCodexChatWireApi,
} from "@/utils/providerConfigUtils";
import {
  providerNeedsRouting,
  supportsOfficialProxyTakeover,
} from "@/utils/providerCapabilities";
import { isOAuthProviderType } from "@/config/constants";

/**
 * Hook for managing provider actions (add, update, delete, switch)
 * Extracts business logic from App.tsx
 */
export function useProviderActions(
  activeApp: AppId,
  isProxyRunning?: boolean,
  isProxyTakeover?: boolean,
) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const addProviderMutation = useAddProviderMutation(activeApp);
  const updateProviderMutation = useUpdateProviderMutation(activeApp);
  const deleteProviderMutation = useDeleteProviderMutation(activeApp);
  const switchProviderMutation = useSwitchProviderMutation(activeApp);

  // Claude 插件同步逻辑
  const syncClaudePlugin = useCallback(
    async (provider: Provider) => {
      if (activeApp !== "claude") return;

      try {
        const settings = await settingsApi.get();
        if (!settings?.enableClaudePluginIntegration) {
          return;
        }

        const isOfficial = provider.category === "official";
        await settingsApi.applyClaudePluginConfig({ official: isOfficial });

        // 静默执行，不显示成功通知
      } catch (error) {
        const detail =
          extractErrorMessage(error) ||
          t("notifications.syncClaudePluginFailed", {
            defaultValue: "同步 Claude 插件失败",
          });
        toast.error(detail, { duration: 4200 });
      }
    },
    [activeApp, t],
  );

  // 添加供应商
  const addProvider = useCallback(
    async (
      provider: Omit<Provider, "id"> & {
        providerKey?: string;
        addToLive?: boolean;
        ensureClaudeDesktopOfficialSeed?: boolean;
        ensureGrokBuildOfficialSeed?: boolean;
        ensureAntigravityOfficialSeed?: boolean;
      },
    ) => {
      const enhanced = injectCodingPlanUsageScript(activeApp, provider);
      await addProviderMutation.mutateAsync(enhanced);
    },
    [addProviderMutation, activeApp, queryClient, t],
  );

  // 更新供应商
  const updateProvider = useCallback(
    async (provider: Provider, originalId?: string) => {
      await updateProviderMutation.mutateAsync({
        provider,
        originalId,
      });

      // 更新托盘菜单（失败不影响主操作）
      try {
        await providersApi.updateTrayMenu();
      } catch (trayError) {
        console.error(
          "Failed to update tray menu after updating provider",
          trayError,
        );
      }
    },
    [updateProviderMutation],
  );

  // 切换供应商
  const switchProvider = useCallback(
    async (provider: Provider) => {
      // OMP 库模式：主按钮「添加」= 把条目（库快照或新建）写回 live 配置
      // （models.yml）。omp 不设「当前供应商」，默认角色在「角色管理」中配置
      // （与 cc-switch opencode 模式一致）。
      if (activeApp === "omp") {
        let ompProvider: OmpProviderConfig | null = null;
        const raw = provider.settingsConfig?.config;
        if (typeof raw === "string" && raw.trim()) {
          try {
            ompProvider = JSON.parse(raw) as OmpProviderConfig;
          } catch {
            ompProvider = null;
          }
        } else if (raw && typeof raw === "object") {
          ompProvider = raw as OmpProviderConfig;
        }
        if (!ompProvider) {
          toast.error(
            t("omp.providerConfigMissing", {
              defaultValue: "供应商配置解析失败，无法添加到配置",
            }),
          );
          return;
        }
        try {
          await ompApi.saveOmpProvider(ompProvider);
          await queryClient.invalidateQueries({
            queryKey: ["providers", "omp"],
          });
          toast.success(
            t("notifications.addToConfigSuccess", {
              defaultValue: "已添加到配置",
            }),
            { closeButton: true },
          );
        } catch (error) {
          console.error(
            "[useProviderActions] omp add to config failed:",
            error,
          );
          toast.error(extractErrorMessage(error));
        }
        return;
      }

      const isCopilotProvider =
        activeApp === "claude" &&
        provider.meta?.providerType === "github_copilot";
      const isCodexChatFormat =
        (activeApp === "codex" || activeApp === "grokbuild") &&
        (provider.meta?.apiFormat === "openai_chat" ||
          (typeof (provider.settingsConfig as Record<string, any>)?.config ===
            "string" &&
            isCodexChatWireApi(
              extractCodexWireApi(
                (provider.settingsConfig as Record<string, any>).config,
              ),
            )));
      const isCodexAnthropicFormat =
        (activeApp === "codex" || activeApp === "grokbuild") &&
        (provider.meta?.apiFormat === "anthropic" ||
          (typeof (provider.settingsConfig as Record<string, any>)?.config ===
            "string" &&
            isCodexAnthropicWireApi(
              extractCodexWireApi(
                (provider.settingsConfig as Record<string, any>).config,
              ),
            )));

      // Claude Desktop 的路由开关就是代理进程本身；其余应用还必须开启当前
      // 应用的 takeover。不能只看全局进程，否则其它应用已接管时会漏判；也
      // 不能只看 takeover，否则 Desktop 在路由已运行时会持续误报。
      const routingReady =
        activeApp === "claude-desktop"
          ? isProxyRunning === true
          : isProxyTakeover === true;

      // Determine why this provider requires the proxy.
      let proxyRequiredReason: string | null = null;
      if (!routingReady && providerNeedsRouting(activeApp, provider)) {
        if (isCopilotProvider) {
          proxyRequiredReason = t("notifications.proxyReasonCopilot", {
            defaultValue: "使用 GitHub Copilot 作为 Claude 供应商",
          });
        } else if (isOAuthProviderType(provider.meta?.providerType)) {
          // 托管 OAuth（codex_oauth / xai_oauth 等）：凭据由本地代理注入，
          // 是否需路由由 providerType 权威决定，不看 apiFormat（后端亦无视，
          // 见 forwarder.rs）——避免 codex_oauth 被改成 anthropic / 旧数据缺省
          // apiFormat 时漏判。Claude 下的 Copilot 保留上面的专属文案。
          proxyRequiredReason = t("notifications.proxyReasonManagedOAuth", {
            defaultValue: "使用托管 OAuth 登录（令牌由本地路由注入）",
          });
        } else if (
          provider.meta?.apiFormat === "openai_chat" &&
          activeApp === "claude"
        ) {
          proxyRequiredReason = t("notifications.proxyReasonOpenAIChat", {
            defaultValue: "使用 OpenAI Chat 接口格式",
          });
        } else if (
          provider.meta?.apiFormat === "openai_responses" &&
          activeApp === "claude"
        ) {
          proxyRequiredReason = t("notifications.proxyReasonOpenAIResponses", {
            defaultValue: "使用 OpenAI Responses 接口格式",
          });
        } else if (isCodexChatFormat) {
          proxyRequiredReason = t("notifications.proxyReasonOpenAIChat", {
            defaultValue: "使用 OpenAI Chat 接口格式",
          });
        } else if (isCodexAnthropicFormat) {
          proxyRequiredReason = t(
            "notifications.proxyReasonAnthropicMessages",
            {
              defaultValue: "使用 Anthropic Messages 接口格式",
            },
          );
        } else if (
          activeApp === "claude-desktop" &&
          provider.meta?.claudeDesktopMode === "proxy"
        ) {
          proxyRequiredReason = t("notifications.proxyReasonClaudeDesktop", {
            defaultValue: "使用 Claude Desktop 本地路由模式",
          });
        } else if (
          provider.meta?.isFullUrl &&
          (activeApp === "claude" ||
            activeApp === "codex" ||
            activeApp === "grokbuild")
        ) {
          proxyRequiredReason = t("notifications.proxyReasonFullUrl", {
            defaultValue: "开启了完整 URL 连接模式",
          });
        } else {
          proxyRequiredReason = t("notifications.proxyReasonRoutingRequired", {
            defaultValue: "需要本地路由处理请求",
          });
        }
      }

      if (proxyRequiredReason) {
        toast.warning(
          t("notifications.proxyRequiredForSwitch", {
            reason: proxyRequiredReason,
            defaultValue:
              "此供应商{{reason}}，需要代理服务才能正常使用，请先启动代理",
          }),
        );
      }

      // Codex official account cards can reuse the active native ChatGPT login
      // through local routing. Other apps' official providers remain blocked.
      const officialSupportsTakeover = supportsOfficialProxyTakeover(
        activeApp,
        provider,
      );
      if (
        isProxyTakeover &&
        provider.category === "official" &&
        !officialSupportsTakeover
      ) {
        toast.error(
          t("notifications.officialBlockedByProxy", {
            defaultValue:
              "代理接管模式下不能切换到官方供应商，使用代理访问官方 API 可能导致账号被封禁",
          }),
          { duration: 6000 },
        );
        return;
      }

      try {
        const result = await switchProviderMutation.mutateAsync(provider.id);
        await syncClaudePlugin(provider);

        // Surface switch warnings by code — a generic "backfill failed"
        // message for an auth-cleanup warning would point the user at the
        // wrong problem entirely.
        if (result?.warnings?.length) {
          const authCleanupFailed = result.warnings.some((warning) =>
            warning.startsWith("codex_auth_cleanup_failed"),
          );
          const hasOtherWarnings = result.warnings.some(
            (warning) => !warning.startsWith("codex_auth_cleanup_failed"),
          );
          if (authCleanupFailed) {
            toast.warning(
              t("notifications.codexAuthCleanupFailed", {
                defaultValue:
                  "切换成功，但未能删除 auth.json，官方登录凭据仍留在磁盘上；如需彻底移除请手动删除 Codex 配置目录中的 auth.json",
              }),
              { duration: 6000 },
            );
          }
          if (hasOtherWarnings) {
            toast.warning(
              t("notifications.backfillWarning", {
                defaultValue:
                  "切换成功，但旧供应商配置回填失败，您手动修改的配置可能未保存",
              }),
              { duration: 5000 },
            );
          }
        }

        // 若已弹过 proxyRequired 警告则不再弹 success
        if (!proxyRequiredReason) {
          let messageKey = "notifications.switchSuccess";
          let defaultMessage = "切换成功！";
          if (activeApp === "codex") {
            messageKey = "notifications.codexRestartRequired";
            defaultMessage = "切换成功，请重启客户端以生效";
          } else if (activeApp === "grokbuild") {
            messageKey = "notifications.grokBuildRestartRequired";
            defaultMessage = "切换成功，请重启 Grok Build 以生效";
          } else if (activeApp === "antigravity") {
            // agy 只从进程环境读 GEMINI_API_KEY / GOOGLE_GEMINI_BASE_URL：
            // 环境变量的增删只对新开的终端 / IDE 生效
            messageKey = "notifications.antigravityRestartRequired";
            defaultMessage =
              "切换成功，环境变量已更新；请重新打开终端 / IDE 以生效";
          } else if (activeApp === "claude-desktop") {
            if (provider.meta?.claudeDesktopMode === "proxy") {
              messageKey = "notifications.claudeDesktopProxyRestartRequired";
              defaultMessage =
                "切换成功，请保持 OGG Switch 运行，并重启 Claude Desktop 后生效";
            } else {
              messageKey = "notifications.claudeDesktopRestartRequired";
              defaultMessage = "切换成功，重启 Claude Desktop 后生效";
            }
          } else if (activeApp === "opencode") {
            messageKey = "notifications.addToConfigSuccess";
            defaultMessage = "已添加到配置";
          }
          toast.success(t(messageKey, { defaultValue: defaultMessage }), {
            closeButton: true,
          });
        }
      } catch {
        // 错误提示由 mutation 处理
      }
    },
    [
      switchProviderMutation,
      syncClaudePlugin,
      activeApp,
      isProxyRunning,
      isProxyTakeover,
      t,
    ],
  );

  // 删除供应商
  const deleteProvider = useCallback(
    async (id: string) => {
      await deleteProviderMutation.mutateAsync(id);
    },
    [deleteProviderMutation],
  );

  // 保存用量脚本
  const saveUsageScript = useCallback(
    async (provider: Provider, script: UsageScript) => {
      try {
        const updatedProvider: Provider = {
          ...provider,
          meta: {
            ...provider.meta,
            usage_script: script,
          },
        };

        if (activeApp === "pi") {
          await piApi.updateProviderUsageScript(provider.id, script);
        } else if (activeApp === "omp") {
          // omp 不在 SQLite，用量脚本真源 = OGG meta store（专道保存）
          await ompApi.updateOmpProviderUsageScript(provider.id, script);
        } else {
          await providersApi.update(updatedProvider, activeApp);
        }
        await queryClient.invalidateQueries({
          queryKey: ["providers", activeApp],
        });
        // 🔧 保存用量脚本后，也应该失效该 provider 的用量查询缓存
        // 这样主页列表会使用新配置重新查询，而不是使用测试时的缓存
        await queryClient.invalidateQueries({
          queryKey: usageKeys.script(provider.id, activeApp),
        });
        await queryClient.invalidateQueries({
          queryKey: ["subscription", "quota", activeApp],
        });
        toast.success(
          t("provider.usageSaved", {
            defaultValue: "用量查询配置已保存",
          }),
          { closeButton: true },
        );
      } catch (error) {
        const detail =
          extractErrorMessage(error) ||
          t("provider.usageSaveFailed", {
            defaultValue: "用量查询配置保存失败",
          });
        toast.error(detail);
      }
    },
    [activeApp, queryClient, t],
  );

  return {
    addProvider,
    updateProvider,
    switchProvider,
    deleteProvider,
    saveUsageScript,
    isLoading:
      addProviderMutation.isPending ||
      updateProviderMutation.isPending ||
      deleteProviderMutation.isPending ||
      switchProviderMutation.isPending,
  };
}
