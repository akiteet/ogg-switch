import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  providersApi,
  sessionsApi,
  settingsApi,
  ompApi,
  type AppId,
} from "@/lib/api";
import type { DeleteSessionOptions } from "@/lib/api/sessions";
import type { SwitchResult } from "@/lib/api/providers";
import type { Provider, SessionMeta, Settings } from "@/types";
import type { OmpProviderConfig } from "@/types/omp";

/**
 * 从通用 Provider 的 settingsConfig 里取出原始 OMP provider 配置。
 * OmpProviderForm 提交时把完整 OmpProviderConfig 存进 config 字段。
 */
const extractOmpProvider = (
  settingsConfig: Record<string, unknown> | undefined,
): OmpProviderConfig | null => {
  const raw = settingsConfig?.config;
  if (typeof raw === "string" && raw.trim()) {
    try {
      return JSON.parse(raw) as OmpProviderConfig;
    } catch {
      return null;
    }
  }
  if (raw && typeof raw === "object") {
    return raw as OmpProviderConfig;
  }
  return null;
};
import {
  extractErrorMessage,
  translatePiProviderMutationError,
} from "@/utils/errorUtils";
import { generateUUID } from "@/utils/uuid";
import { proxyKeys } from "@/lib/query/proxy";
import { usageKeys } from "@/lib/query/usage";
import { invalidatePiProviderCaches } from "@/lib/query/pi";
import {
  GROKBUILD_OFFICIAL_PROVIDER_ID,
  ANTIGRAVITY_OFFICIAL_PROVIDER_ID,
} from "@/utils/providerCapabilities";

export const useAddProviderMutation = (appId: AppId) => {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: async (
      providerInput: Omit<Provider, "id"> & {
        providerKey?: string;
        addToLive?: boolean;
        ensureClaudeDesktopOfficialSeed?: boolean;
        ensureGrokBuildOfficialSeed?: boolean;
        ensureAntigravityOfficialSeed?: boolean;
      },
    ) => {
      const {
        providerKey: _providerKey,
        addToLive,
        ensureClaudeDesktopOfficialSeed,
        ensureGrokBuildOfficialSeed,
        ensureAntigravityOfficialSeed,
        ...rest
      } = providerInput;

      if (appId === "claude-desktop" && ensureClaudeDesktopOfficialSeed) {
        await providersApi.ensureClaudeDesktopOfficialProvider();
        const providers = await providersApi.getAll(appId);
        const officialProvider = providers["claude-desktop-official"];
        if (!officialProvider) {
          throw new Error("Claude Desktop official provider was not created");
        }
        return officialProvider;
      }

      if (appId === "grokbuild" && ensureGrokBuildOfficialSeed) {
        await providersApi.ensureGrokBuildOfficialProvider();
        const providers = await providersApi.getAll(appId);
        const officialProvider = providers[GROKBUILD_OFFICIAL_PROVIDER_ID];
        if (!officialProvider) {
          throw new Error("Grok Build official provider was not created");
        }
        return officialProvider;
      }

      if (appId === "antigravity" && ensureAntigravityOfficialSeed) {
        await providersApi.ensureAntigravityOfficialProvider();
        const providers = await providersApi.getAll(appId);
        const officialProvider = providers[ANTIGRAVITY_OFFICIAL_PROVIDER_ID];
        if (!officialProvider) {
          throw new Error("Antigravity official provider was not created");
        }
        return officialProvider;
      }

      // Oh My Pi：直接写本机 ~/.omp/agent 的 YAML，不走通用 SQLite 供应商表。
      if (appId === "omp") {
        const ompProvider = extractOmpProvider(rest.settingsConfig);
        if (!ompProvider) {
          throw new Error(
            t("omp.providerConfigMissing", {
              defaultValue: "供应商配置解析失败，无法添加到配置",
            }),
          );
        }
        await ompApi.saveOmpProvider(ompProvider);
        return {
          ...rest,
          id: ompProvider.id,
          name: ompProvider.name,
          createdAt: Date.now(),
        } as Provider;
      }

      let id: string;

      if (appId === "opencode" || appId === "pi") {
        if (
          providerInput.category === "omo" ||
          providerInput.category === "omo-slim"
        ) {
          const prefix = providerInput.category === "omo" ? "omo" : "omo-slim";
          id = `${prefix}-${generateUUID()}`;
        } else {
          if (!providerInput.providerKey) {
            throw new Error(`Provider key is required for ${appId}`);
          }
          id = providerInput.providerKey;
        }
      } else {
        id = generateUUID();
      }

      const newProvider: Provider = {
        ...rest,
        id,
        createdAt: Date.now(),
      };
      delete (newProvider as any).providerKey;

      await providersApi.add(newProvider, appId, addToLive);
      return newProvider;
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["providers", appId] });

      if (appId === "opencode") {
        await queryClient.invalidateQueries({
          queryKey: ["omo", "current-provider-id"],
        });
        await queryClient.invalidateQueries({
          queryKey: ["omo", "provider-count"],
        });
        await queryClient.invalidateQueries({
          queryKey: ["omo-slim", "current-provider-id"],
        });
        await queryClient.invalidateQueries({
          queryKey: ["omo-slim", "provider-count"],
        });
      }

      try {
        await providersApi.updateTrayMenu();
      } catch (trayError) {
        console.error(
          "Failed to update tray menu after adding provider",
          trayError,
        );
      }

      toast.success(
        t("notifications.providerAdded", {
          defaultValue: "供应商已添加",
        }),
        {
          closeButton: true,
        },
      );
    },
    onError: (error: Error) => {
      const rawDetail = extractErrorMessage(error);
      const detail =
        (appId === "pi"
          ? translatePiProviderMutationError(rawDetail, t)
          : "") ||
        rawDetail ||
        t("common.unknown");
      toast.error(
        t("notifications.addFailed", {
          defaultValue: "添加供应商失败: {{error}}",
          error: detail,
        }),
      );
    },
    onSettled: async () => {
      if (appId === "pi") {
        await invalidatePiProviderCaches(queryClient);
      }
    },
  });
};

export const useUpdateProviderMutation = (appId: AppId) => {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: async ({
      provider,
      originalId,
    }: {
      provider: Provider;
      originalId?: string;
    }) => {
      // Oh My Pi：写回本机 ~/.omp/agent YAML，不走通用 SQLite 表。
      if (appId === "omp") {
        const ompProvider = extractOmpProvider(provider.settingsConfig);
        if (ompProvider) {
          await ompApi.saveOmpProvider(ompProvider);
        }
        return provider;
      }
      await providersApi.update(provider, appId, originalId);
      return provider;
    },
    onSuccess: async (provider, variables) => {
      await queryClient.invalidateQueries({ queryKey: ["providers", appId] });
      await queryClient.invalidateQueries({
        queryKey: usageKeys.script(provider.id, appId),
      });
      if (variables.originalId && variables.originalId !== provider.id) {
        await queryClient.invalidateQueries({
          queryKey: usageKeys.script(variables.originalId, appId),
        });
      }
      toast.success(
        t("notifications.updateSuccess", {
          defaultValue: "供应商更新成功",
        }),
        {
          closeButton: true,
        },
      );
    },
    onError: (error: Error) => {
      const rawDetail = extractErrorMessage(error);
      const detail =
        (appId === "pi"
          ? translatePiProviderMutationError(rawDetail, t)
          : "") ||
        rawDetail ||
        t("common.unknown");
      toast.error(
        t("notifications.updateFailed", {
          defaultValue: "更新供应商失败: {{error}}",
          error: detail,
        }),
      );
    },
    onSettled: async () => {
      if (appId === "pi") {
        await invalidatePiProviderCaches(queryClient);
      }
    },
  });
};

export const useDeleteProviderMutation = (appId: AppId) => {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: async (providerId: string) => {
      // Oh My Pi：从本机 ~/.omp/agent YAML 删除，不走通用 SQLite 表。
      if (appId === "omp") {
        await ompApi.deleteOmpProvider(providerId);
        return;
      }
      await providersApi.delete(providerId, appId);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["providers", appId] });

      if (appId === "opencode") {
        await queryClient.invalidateQueries({
          queryKey: ["omo", "current-provider-id"],
        });
        await queryClient.invalidateQueries({
          queryKey: ["omo", "provider-count"],
        });
        await queryClient.invalidateQueries({
          queryKey: ["omo-slim", "current-provider-id"],
        });
        await queryClient.invalidateQueries({
          queryKey: ["omo-slim", "provider-count"],
        });
      }

      try {
        await providersApi.updateTrayMenu();
      } catch (trayError) {
        console.error(
          "Failed to update tray menu after deleting provider",
          trayError,
        );
      }

      toast.success(
        t("notifications.deleteSuccess", {
          defaultValue: "供应商已删除",
        }),
        {
          closeButton: true,
        },
      );
    },
    onError: (error: Error) => {
      const rawDetail = extractErrorMessage(error);
      const detail =
        (appId === "pi"
          ? translatePiProviderMutationError(rawDetail, t)
          : "") ||
        rawDetail ||
        t("common.unknown");
      toast.error(
        t("notifications.deleteFailed", {
          defaultValue: "删除供应商失败: {{error}}",
          error: detail,
        }),
      );
    },
    onSettled: async () => {
      if (appId === "pi") {
        await invalidatePiProviderCaches(queryClient);
      }
    },
  });
};

export const useSwitchProviderMutation = (appId: AppId) => {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: async (providerId: string): Promise<SwitchResult> => {
      return await providersApi.switch(providerId, appId);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["providers", appId] });
      if (appId === "claude-desktop") {
        await queryClient.invalidateQueries({ queryKey: proxyKeys.status });
        await queryClient.invalidateQueries({
          queryKey: ["claudeDesktopStatus"],
        });
      }

      // OpenCode/Pi: also invalidate live provider IDs cache to update button state
      if (appId === "opencode") {
        await queryClient.invalidateQueries({
          queryKey: ["opencodeLiveProviderIds"],
        });
        await queryClient.invalidateQueries({
          queryKey: ["opencode", "runtime-models"],
        });
        await queryClient.invalidateQueries({
          queryKey: ["omo", "current-provider-id"],
        });
        await queryClient.invalidateQueries({
          queryKey: ["omo-slim", "current-provider-id"],
        });
      }
      try {
        await providersApi.updateTrayMenu();
      } catch (trayError) {
        console.error(
          "Failed to update tray menu after switching provider",
          trayError,
        );
      }
    },
    onError: (error: Error) => {
      const detail = extractErrorMessage(error) || t("common.unknown");

      toast.error(
        t("notifications.switchFailedTitle", { defaultValue: "切换失败" }),
        {
          description: t("notifications.switchFailed", {
            defaultValue: "切换失败：{{error}}",
            error: detail,
          }),
          duration: 6000,
          action: {
            label: t("common.copy", { defaultValue: "复制" }),
            onClick: () => {
              navigator.clipboard?.writeText(detail).catch(() => undefined);
            },
          },
        },
      );
    },
    onSettled: async () => {
      if (appId === "pi") {
        await invalidatePiProviderCaches(queryClient);
      }
    },
  });
};

export const useDeleteSessionMutation = () => {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: async (input: DeleteSessionOptions) => {
      await sessionsApi.delete(input);
      return input;
    },
    onSuccess: async (input) => {
      queryClient.setQueryData<SessionMeta[]>(["sessions"], (current) =>
        (current ?? []).filter(
          (session) =>
            !(
              session.providerId === input.providerId &&
              session.sessionId === input.sessionId &&
              session.sourcePath === input.sourcePath
            ),
        ),
      );
      queryClient.removeQueries({
        queryKey: ["sessionMessages", input.providerId, input.sourcePath],
      });

      await queryClient.invalidateQueries({ queryKey: ["sessions"] });

      toast.success(
        t("sessionManager.sessionDeleted", {
          defaultValue: "会话已删除",
        }),
      );
    },
    onError: (error: Error) => {
      const detail = extractErrorMessage(error) || t("common.unknown");
      toast.error(
        t("sessionManager.deleteFailed", {
          defaultValue: "删除会话失败: {{error}}",
          error: detail,
        }),
      );
    },
  });
};

export const useSaveSettingsMutation = () => {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (settings: Settings) => {
      await settingsApi.save(settings);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      await queryClient.invalidateQueries({
        queryKey: ["opencode", "runtime-models"],
      });
    },
  });
};
