import { useCallback, useEffect, useMemo, useState } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Info, KeyRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { providerSchema, type ProviderFormData } from "@/lib/schemas/provider";
import type { ProviderCategory } from "@/types";
import type { ProviderFormProps } from "./ProviderForm";
import { BasicFormFields } from "./BasicFormFields";
import { ProviderPresetSelector } from "./ProviderPresetSelector";
import { ApiKeySection, ModelInputWithFetch } from "./shared";
import {
  antigravityProviderPresets,
  type AntigravityProviderPreset,
} from "@/config/antigravityProviderPresets";
import { resolveProviderIcon } from "@/utils/providerIcon";
import { ANTIGRAVITY_OFFICIAL_PROVIDER_ID } from "@/utils/providerCapabilities";
import {
  fetchModelsForConfig,
  showFetchModelsError,
  type FetchedModel,
} from "@/lib/api/model-fetch";

type AntigravityProviderFormProps = Omit<ProviderFormProps, "appId">;

const antigravityPresetEntries: Array<{
  id: string;
  preset: AntigravityProviderPreset;
}> = antigravityProviderPresets.map((preset, index) => ({
  id: index === 0 ? ANTIGRAVITY_OFFICIAL_PROVIDER_ID : `antigravity-${index}`,
  preset,
}));

/**
 * Antigravity (agy) 供应商表单。
 *
 * 两种条目形态：
 * - 官方（oauth 无 token）：仅清 API key 配置，不接管 agy 登录态；
 * - API key / 中转站：settings.json modelProvider + 持久环境变量。
 * Google 账号在认证中心独立账号池里，不是供应商。
 */
export function AntigravityProviderForm({
  submitLabel,
  onSubmit,
  onCancel,
  onSubmittingChange,
  initialData,
  showButtons = true,
}: AntigravityProviderFormProps) {
  const { t } = useTranslation();
  const initialAuthType =
    (initialData?.settingsConfig?.authType as string | undefined) ?? "api-key";

  const [selectedPresetId, setSelectedPresetId] = useState<string | null>(
    initialData ? null : "custom",
  );
  const [category, setCategory] = useState<ProviderCategory | undefined>(
    initialData?.category ?? "custom",
  );
  const [isOfficialOAuth, setIsOfficialOAuth] = useState(
    initialAuthType === "oauth",
  );
  const [fetchedModels, setFetchedModels] = useState<FetchedModel[]>([]);
  const [isFetchingModels, setIsFetchingModels] = useState(false);
  const [baseUrl, setBaseUrl] = useState(() => {
    const env = initialData?.settingsConfig?.env as
      | Record<string, unknown>
      | undefined;
    return typeof env?.GOOGLE_GEMINI_BASE_URL === "string"
      ? env.GOOGLE_GEMINI_BASE_URL
      : "";
  });
  const [apiKey, setApiKey] = useState(() => {
    const env = initialData?.settingsConfig?.env as
      | Record<string, unknown>
      | undefined;
    return typeof env?.GEMINI_API_KEY === "string" ? env.GEMINI_API_KEY : "";
  });
  const [defaultModel, setDefaultModel] = useState(() => {
    const env = initialData?.settingsConfig?.env as
      | Record<string, unknown>
      | undefined;
    return typeof env?.GEMINI_MODEL === "string" ? env.GEMINI_MODEL : "";
  });
  const [extraEnvText, setExtraEnvText] = useState(() => {
    const env = initialData?.settingsConfig?.env as
      | Record<string, unknown>
      | undefined;
    const reserved = new Set([
      "GEMINI_API_KEY",
      "GOOGLE_GEMINI_BASE_URL",
      "GEMINI_MODEL",
    ]);
    return Object.entries(env ?? {})
      .filter(([key]) => !reserved.has(key))
      .map(([key, value]) => `${key}=${String(value)}`)
      .join("\n");
  });

  const form = useForm<ProviderFormData>({
    resolver: zodResolver(providerSchema),
    defaultValues: {
      name: initialData?.name ?? "",
      websiteUrl: initialData?.websiteUrl ?? "",
      notes: initialData?.notes ?? "",
      settingsConfig: initialData
        ? JSON.stringify(initialData.settingsConfig)
        : JSON.stringify({ authType: "api-key", env: {} }),
      icon:
        resolveProviderIcon(
          "antigravity",
          initialData?.icon,
          initialData?.iconColor,
        ) ?? "",
      iconColor: initialData?.iconColor ?? "",
    },
    mode: "onSubmit",
  });
  const { isSubmitting } = form.formState;
  const websiteUrl = form.watch("websiteUrl") ?? "";

  useEffect(() => {
    onSubmittingChange?.(isSubmitting);
  }, [isSubmitting, onSubmittingChange]);

  const presetCategoryLabels = useMemo(
    () => ({
      official: t("providerForm.categoryOfficial", { defaultValue: "官方" }),
      aggregator: t("providerForm.categoryAggregation", {
        defaultValue: "聚合服务",
      }),
      third_party: t("providerForm.categoryThirdParty", {
        defaultValue: "第三方",
      }),
    }),
    [t],
  );

  const handlePresetChange = (presetId: string) => {
    setSelectedPresetId(presetId);
    if (presetId === "custom") {
      setCategory("custom");
      setIsOfficialOAuth(false);
      setBaseUrl("");
      setApiKey("");
      setDefaultModel("");
      setExtraEnvText("");
      return;
    }

    const entry = antigravityPresetEntries.find(
      (candidate) => candidate.id === presetId,
    );
    if (!entry) return;
    const preset = entry.preset;
    const presetName = preset.nameKey ? String(t(preset.nameKey)) : preset.name;
    form.setValue("name", presetName);
    form.setValue("websiteUrl", preset.websiteUrl ?? "");
    form.setValue("icon", preset.icon ?? "");
    form.setValue("iconColor", preset.iconColor ?? "");
    setCategory(preset.category ?? "custom");
    setIsOfficialOAuth(preset.authType === "oauth");
    setBaseUrl(preset.env?.GOOGLE_GEMINI_BASE_URL ?? "");
    setApiKey(preset.env?.GEMINI_API_KEY ?? "");
    setDefaultModel(preset.env?.GEMINI_MODEL ?? "");
    const presetReserved = new Set([
      "GEMINI_API_KEY",
      "GOOGLE_GEMINI_BASE_URL",
      "GEMINI_MODEL",
    ]);
    setExtraEnvText(
      Object.entries(preset.env ?? {})
        .filter(([key]) => !presetReserved.has(key))
        .map(([key, value]) => `${key}=${String(value)}`)
        .join("\n"),
    );
  };

  const handleFetchModels = useCallback(() => {
    const endpoint =
      baseUrl.trim().replace(/\/+$/, "") ||
      "https://generativelanguage.googleapis.com/v1beta/openai";
    if (!apiKey.trim()) {
      showFetchModelsError(null, t, {
        hasApiKey: false,
        hasBaseUrl: true,
      });
      return;
    }
    setIsFetchingModels(true);
    fetchModelsForConfig(endpoint, apiKey.trim())
      .then((models) => {
        setFetchedModels(models);
        if (models.length === 0) {
          toast.info(t("providerForm.fetchModelsEmpty"));
        } else {
          toast.success(
            t("providerForm.fetchModelsSuccess", { count: models.length }),
          );
        }
      })
      .catch((err) => {
        console.warn("[ModelFetch] Failed:", err);
        showFetchModelsError(err, t);
      })
      .finally(() => setIsFetchingModels(false));
  }, [apiKey, baseUrl, t]);

  const parseExtraEnv = (text: string): Record<string, string> | null => {
    const entries: Record<string, string> = {};
    for (const line of text.split("\n")) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      const eq = trimmed.indexOf("=");
      if (eq <= 0) return null;
      const key = trimmed.slice(0, eq).trim();
      const value = trimmed.slice(eq + 1).trim();
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) return null;
      entries[key] = value;
    }
    return entries;
  };

  const handleSubmit = async (values: ProviderFormData) => {
    const name = values.name.trim();
    if (!name) {
      form.setError("name", {
        message: t("providerForm.nameRequired", {
          defaultValue: "请填写供应商名称",
        }),
      });
      return;
    }

    // 官方条目：oauth 无 token（新增走 ensure seed，编辑只允许改元信息）
    if (isOfficialOAuth) {
      await onSubmit({
        ...values,
        name,
        websiteUrl: values.websiteUrl?.trim() ?? "",
        notes: values.notes?.trim() ?? "",
        settingsConfig: JSON.stringify({ authType: "oauth" }),
        presetId: selectedPresetId ?? undefined,
        presetCategory: category ?? "custom",
        isPartner: false,
        meta: initialData?.meta,
      });
      return;
    }

    // API key / 中转站
    if (!apiKey.trim()) {
      toast.error(
        t("providerForm.apiKeyRequired", {
          defaultValue: "非官方供应商请填写 API Key",
        }),
      );
      return;
    }

    const extraEnv = parseExtraEnv(extraEnvText);
    if (extraEnv === null) {
      toast.error(
        t("provider.form.antigravity.extraEnvInvalid", {
          defaultValue:
            "额外环境变量格式错误：每行一条 KEY=VALUE，变量名只能含字母/数字/下划线",
        }),
      );
      return;
    }

    const env: Record<string, string> = {
      GEMINI_API_KEY: apiKey.trim(),
      ...extraEnv,
    };
    if (baseUrl.trim()) {
      env.GOOGLE_GEMINI_BASE_URL = baseUrl.trim().replace(/\/+$/, "");
    }
    if (defaultModel.trim()) {
      env.GEMINI_MODEL = defaultModel.trim();
    }
    await onSubmit({
      ...values,
      name,
      websiteUrl: values.websiteUrl?.trim() ?? "",
      notes: values.notes?.trim() ?? "",
      settingsConfig: JSON.stringify({ authType: "api-key", env }),
      presetId: selectedPresetId ?? undefined,
      presetCategory: category ?? "custom",
      isPartner: false,
      meta: initialData?.meta,
    });
  };

  return (
    <Form {...form}>
      <form
        id="provider-form"
        onSubmit={form.handleSubmit(handleSubmit)}
        className="space-y-6 glass rounded-xl p-6 border border-white/10"
      >
        {!initialData && (
          <ProviderPresetSelector
            selectedPresetId={selectedPresetId}
            presetEntries={antigravityPresetEntries}
            presetCategoryLabels={presetCategoryLabels}
            onPresetChange={handlePresetChange}
            category={category}
          />
        )}

        <BasicFormFields form={form} />

        {isOfficialOAuth ? (
          <div className="rounded-lg border border-blue-200 bg-blue-50 p-4 dark:border-blue-800 dark:bg-blue-950">
            <div className="flex gap-3">
              <Info className="h-5 w-5 flex-shrink-0 text-blue-600 dark:text-blue-400" />
              <div className="space-y-1">
                <p className="text-sm font-medium text-blue-900 dark:text-blue-100">
                  {t("provider.form.antigravity.oauthTitle", {
                    defaultValue: "Google 登录模式",
                  })}
                </p>
                <p className="text-sm text-blue-700 dark:text-blue-300">
                  {t("provider.form.antigravity.oauthHint", {
                    defaultValue:
                      "清除 API Key 配置并回落到 agy 自身的 Google 登录态；不会改动 agy 已有的登录凭据文件。",
                  })}
                </p>
              </div>
            </div>
          </div>
        ) : (
          <>
            <ApiKeySection
              value={apiKey}
              onChange={setApiKey}
              category={category}
              shouldShowLink={Boolean(websiteUrl)}
              websiteUrl={websiteUrl}
            />

            {/* Base URL：留空 = Gemini 官方直连；中转站填 Gemini 兼容端点 */}
            <div className="space-y-2">
              <FormLabel htmlFor="antigravity-baseUrl">
                {t("provider.form.antigravity.baseUrlLabel", {
                  defaultValue: "API 端点（选填）",
                })}
              </FormLabel>
              <div className="relative">
                <KeyRound className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
                <Input
                  id="antigravity-baseUrl"
                  value={baseUrl}
                  onChange={(event) => setBaseUrl(event.target.value)}
                  placeholder={t(
                    "provider.form.antigravity.baseUrlPlaceholder",
                    {
                      defaultValue: "Gemini 兼容端点，留空 = Google 官方直连",
                    },
                  )}
                  className="pl-9"
                  autoComplete="off"
                />
              </div>
              <p className="text-xs text-muted-foreground">
                {t("provider.form.antigravity.baseUrlHint", {
                  defaultValue:
                    "agy 只识别进程环境变量：切换后需重开终端 / IDE 才会读到新端点。",
                })}
              </p>
            </div>

            {/* 默认模型（GEMINI_MODEL）：agy 是否读取未经官方确认 */}
            <div className="space-y-2">
              <FormLabel htmlFor="antigravity-default-model">
                {t("provider.form.antigravity.defaultModelLabel", {
                  defaultValue: "默认模型（选填）",
                })}
              </FormLabel>
              <ModelInputWithFetch
                id="antigravity-default-model"
                value={defaultModel}
                onChange={setDefaultModel}
                placeholder={t(
                  "provider.form.antigravity.defaultModelPlaceholder",
                  {
                    defaultValue:
                      "如 gemini-3-pro；留空 = 在 agy 内 /model 选择",
                  },
                )}
                fetchedModels={fetchedModels}
                isLoading={isFetchingModels}
                onFetch={handleFetchModels}
              />
              <p className="text-xs text-muted-foreground">
                {t("provider.form.antigravity.defaultModelHint", {
                  defaultValue:
                    "写入 GEMINI_MODEL 环境变量（Gemini CLI 惯例）；agy 未官方声明读取该变量，以实际为准。",
                })}
              </p>
            </div>

            {/* 额外环境变量：任意 KEY=VALUE，随供应商持久化 */}
            <div className="space-y-2">
              <FormLabel htmlFor="antigravity-extra-env">
                {t("provider.form.antigravity.extraEnvLabel", {
                  defaultValue: "额外环境变量（可选）",
                })}
              </FormLabel>
              <Textarea
                id="antigravity-extra-env"
                value={extraEnvText}
                onChange={(event) => setExtraEnvText(event.target.value)}
                placeholder={"KEY=VALUE\nANOTHER_VAR=value"}
                rows={3}
                className="font-mono text-xs"
                spellCheck={false}
              />
              <p className="text-xs text-muted-foreground">
                {t("provider.form.antigravity.extraEnvHint", {
                  defaultValue:
                    "每行一条 KEY=VALUE，随切换写入持久环境变量；清空该变量值即删除。",
                })}
              </p>
            </div>
          </>
        )}

        <FormField
          control={form.control}
          name="settingsConfig"
          render={() => (
            <FormItem className="hidden">
              <FormControl>
                <Input type="hidden" />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />

        {showButtons && (
          <div className="flex justify-end gap-2">
            <Button variant="outline" type="button" onClick={onCancel}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={isSubmitting}>
              {submitLabel}
            </Button>
          </div>
        )}
      </form>
    </Form>
  );
}
