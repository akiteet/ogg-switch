import { useEffect, useMemo, useState } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
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
import JsonEditor from "@/components/JsonEditor";
import { useDarkMode } from "@/hooks/useDarkMode";
import { providerSchema, type ProviderFormData } from "@/lib/schemas/provider";
import {
  buildLocalProxyRequestOverrides,
  formatRequestOverrideObject,
} from "@/lib/requestOverrides";
import type {
  ClaudeApiKeyField,
  CodexApiFormat,
  CodexChatReasoning,
  PromptCacheRoutingMode,
  ProviderCategory,
  ProviderMeta,
} from "@/types";
import type { ProviderFormProps, ProviderFormValues } from "./ProviderForm";
import { BasicFormFields } from "./BasicFormFields";
import { CodexFormFields } from "./CodexFormFields";
import { GrokModelEntriesEditor } from "./GrokModelEntriesEditor";
import { ProviderPresetSelector } from "./ProviderPresetSelector";
import {
  grokBuildOfficialPreset,
  grokBuildProviderPresets,
  type GrokBuildProviderPreset,
} from "@/config/grokBuildProviderPresets";
import {
  codexApiFormatFromWireApi,
  extractCodexBaseUrl,
  extractCodexModelName,
  extractCodexWireApi,
} from "@/utils/providerConfigUtils";
import {  buildGrokBuildConfig,
  GROK_BUILD_DEFAULT_API_BACKEND,
  parseGrokBuildConfig,
  validateGrokBuildConfig,
  parseGrokModelEntries,
  updateGrokModelEntries,
  validateGrokModelEntries,
  type GrokModelEntry,
} from "@/utils/grokBuildConfig";
import { resolveProviderIcon } from "@/utils/providerIcon";
import { GROKBUILD_OFFICIAL_PROVIDER_ID } from "@/utils/providerCapabilities";

type GrokBuildProviderFormProps = Omit<ProviderFormProps, "appId">;

// 预设列表见 grokBuildProviderPresets.ts：独立维护（与 Codex 预设无联动），
// 不含官方 / OAuth / 国产官方直连 / 纯开源托管站，默认模型为 Grok 系。
const grokPresetEntries: Array<{
  id: string;
  preset: GrokBuildProviderPreset;
}> = [
  { id: GROKBUILD_OFFICIAL_PROVIDER_ID, preset: grokBuildOfficialPreset },
  ...grokBuildProviderPresets.map((preset, index) => ({
    id: `grokbuild-${index}`,
    preset,
  })),
];

export function GrokBuildProviderForm({
  providerId,
  submitLabel,
  onSubmit,
  onCancel,
  onSubmittingChange,
  initialData,
  showButtons = true,
}: GrokBuildProviderFormProps) {
  const { t } = useTranslation();
  const isDarkMode = useDarkMode();
  const initialConfigText =
    typeof initialData?.settingsConfig?.config === "string"
      ? initialData.settingsConfig.config
      : undefined;
  const initialConfig = useMemo(
    () => parseGrokBuildConfig(initialConfigText, initialData?.name),
    [initialConfigText, initialData?.name],
  );

  const [selectedPresetId, setSelectedPresetId] = useState<string | null>(
    initialData ? null : "custom",
  );
  const [category, setCategory] = useState<ProviderCategory | undefined>(
    initialData?.category ?? "custom",
  );
  const [isPartner, setIsPartner] = useState(
    initialData?.meta?.isPartner ?? false,
  );
  const [partnerPromotionKey, setPartnerPromotionKey] = useState<string>();
  const [upstreamModel, setUpstreamModel] = useState(
    initialConfig.upstreamModel ?? initialConfig.model,
  );
  const [baseUrl, setBaseUrl] = useState(initialConfig.baseUrl);
  const [apiKey, setApiKey] = useState(initialConfig.apiKey);
  const [contextWindow, setContextWindow] = useState(
    String(initialConfig.contextWindow),
  );
  const [modelEntries, setModelEntries] = useState<GrokModelEntry[]>(
    () => parseGrokModelEntries(initialConfigText ?? buildGrokBuildConfig(initialConfig)).entries,
  );
  const [defaultModelKey, setDefaultModelKey] = useState<string>(
    () => parseGrokModelEntries(initialConfigText ?? buildGrokBuildConfig(initialConfig)).defaultKey,
  );
  // 默认模型只有一份真相：defaultModelKey。
  // 此前另有 profile state 与之并存，两者只在部分路径同步，导致“默认行被改了
  // ID / 被删除”时它与条目列表脱节（保存时校验报“默认模型不在列表里”）。
  const profile = defaultModelKey;
  const [rawConfig, setRawConfig] = useState(
    initialConfigText ?? buildGrokBuildConfig(initialConfig),
  );
  const [apiFormat, setApiFormat] = useState<CodexApiFormat>(
    (initialData?.meta?.apiFormat as CodexApiFormat | undefined) ??
      "openai_responses",
  );
  const [anthropicAuthField, setAnthropicAuthField] =
    useState<ClaudeApiKeyField>(
      initialData?.meta?.apiKeyField ?? "ANTHROPIC_AUTH_TOKEN",
    );
  const [impersonateClaudeCode, setImpersonateClaudeCode] = useState(
    initialData?.meta?.impersonateClaudeCode === true,
  );
  const [maxOutputTokens, setMaxOutputTokens] = useState(
    initialData?.meta?.maxOutputTokens
      ? String(initialData.meta.maxOutputTokens)
      : "",
  );
  const [codexChatReasoning, setCodexChatReasoning] =
    useState<CodexChatReasoning>(initialData?.meta?.codexChatReasoning ?? {});
  const [promptCacheRouting, setPromptCacheRouting] =
    useState<PromptCacheRoutingMode>(
      initialData?.meta?.promptCacheRouting ?? "auto",
    );
  const [isFullUrl, setIsFullUrl] = useState(
    initialData?.meta?.isFullUrl ?? false,
  );
  const [customUserAgent, setCustomUserAgent] = useState(
    initialData?.meta?.customUserAgent ?? "",
  );
  const [headersOverride, setHeadersOverride] = useState(
    formatRequestOverrideObject(
      initialData?.meta?.localProxyRequestOverrides?.headers,
    ),
  );
  const [bodyOverride, setBodyOverride] = useState(
    formatRequestOverrideObject(
      initialData?.meta?.localProxyRequestOverrides?.body,
    ),
  );
  const [endpointAutoSelect, setEndpointAutoSelect] = useState(
    initialData?.meta?.endpointAutoSelect ?? true,
  );
  const [isEndpointModalOpen, setIsEndpointModalOpen] = useState(false);
  const [presetEndpoints, setPresetEndpoints] = useState<string[]>([]);
  const [draftCustomEndpoints, setDraftCustomEndpoints] = useState<string[]>(
    [],
  );

  const form = useForm<ProviderFormData>({
    resolver: zodResolver(providerSchema),
    defaultValues: {
      name: initialData?.name ?? "",
      websiteUrl: initialData?.websiteUrl ?? "",
      notes: initialData?.notes ?? "",
      settingsConfig: JSON.stringify({ config: rawConfig }),
      icon:
        resolveProviderIcon(
          "grokbuild",
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

  // Grok Build 预设已不含 cn_official（国产官方直连无法在 Grok CLI 使用）
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

  const speedTestEndpoints = useMemo(() => {
    const urls = new Set<string>();
    const add = (url?: string) => {
      const normalized = url?.trim().replace(/\/+$/, "");
      if (normalized) urls.add(normalized);
    };
    add(baseUrl);
    presetEndpoints.forEach(add);
    draftCustomEndpoints.forEach(add);
    return Array.from(urls).map((url) => ({ url }));
  }, [baseUrl, draftCustomEndpoints, presetEndpoints]);

  // 统一的配置重建:模型条目 + 供应商级 URL/Key 一起写入,URL/Key 会盖到每张表。
  const rebuildConfig = (
    overrides: {
      entries?: GrokModelEntry[];
      defaultKey?: string;
      baseUrl?: string;
      apiKey?: string;
    } = {},
  ) => {
    const nextEntries = overrides.entries ?? modelEntries;
    const nextDefault = overrides.defaultKey ?? defaultModelKey;
    const nextBaseUrl = overrides.baseUrl ?? baseUrl;
    const nextApiKey = overrides.apiKey ?? apiKey;
    setRawConfig((current) =>
      updateGrokModelEntries(
        current,
        {
          entries: nextEntries,
          defaultKey: nextDefault,
          modelsExtra: parseGrokModelEntries(current).modelsExtra,
        },
        { baseUrl: nextBaseUrl, apiKey: nextApiKey, apiBackend: GROK_BUILD_DEFAULT_API_BACKEND },
      ),
    );
  };

  const handlePresetChange = (presetId: string) => {
    setSelectedPresetId(presetId);
    if (presetId === "custom") {
      setCategory("custom");
      setIsPartner(false);
      setPartnerPromotionKey(undefined);
      setPresetEndpoints([]);
      return;
    }

    if (presetId === GROKBUILD_OFFICIAL_PROVIDER_ID) {
      // 官方登录：无 API Key / 地址 / 模型表可填，提交走 ensure seed 流程
      form.setValue("name", grokBuildOfficialPreset.name);
      form.setValue("websiteUrl", grokBuildOfficialPreset.websiteUrl);
      form.setValue("icon", grokBuildOfficialPreset.icon ?? "");
      form.setValue("iconColor", grokBuildOfficialPreset.iconColor ?? "");
      setCategory("official");
      setIsPartner(false);
      setPartnerPromotionKey(undefined);
      setPresetEndpoints([]);
      setRawConfig("");
      return;
    }

    const entry = grokPresetEntries.find(
      (candidate) => candidate.id === presetId,
    );
    if (!entry) return;
    const preset = entry.preset;
    const presetName = preset.nameKey ? String(t(preset.nameKey)) : preset.name;
    const presetBaseUrl = extractCodexBaseUrl(preset.config) ?? "";
    const presetModel = extractCodexModelName(preset.config) ?? profile;
    const presetApiFormat =
      preset.apiFormat ??
      codexApiFormatFromWireApi(extractCodexWireApi(preset.config)) ??
      "openai_responses";
    const presetApiKey =
      "auth" in preset && typeof preset.auth?.OPENAI_API_KEY === "string"
        ? preset.auth.OPENAI_API_KEY
        : "";
    form.setValue("name", presetName);
    form.setValue("websiteUrl", preset.websiteUrl ?? "");
    form.setValue("icon", preset.icon ?? "");
    form.setValue("iconColor", preset.iconColor ?? "");
    setCategory(preset.category ?? "custom");
    setIsPartner(preset.isPartner ?? false);
    setPartnerPromotionKey(preset.partnerPromotionKey);
    setBaseUrl(presetBaseUrl);
    setApiKey(presetApiKey);
    setUpstreamModel(presetModel);
    setApiFormat(presetApiFormat);
    setPresetEndpoints(preset.endpointCandidates ?? []);
    setRawConfig(
      buildGrokBuildConfig({
        model: profile,
        upstreamModel: presetModel,
        baseUrl: presetBaseUrl,
        name: presetName,
        apiKey: presetApiKey,
        apiBackend: GROK_BUILD_DEFAULT_API_BACKEND,
        contextWindow: Number.parseInt(contextWindow, 10),
      }),
    );
  };

  const handleRawConfigChange = (value: string) => {
    setRawConfig(value);
    if (validateGrokBuildConfig(value)) return;
    const parsed = parseGrokBuildConfig(value, form.getValues("name"));
    // 手工编辑 TOML 后，条目列表与默认值都要跟着文本走，
    // 否则界面仍显示旧条目、下次保存会把用户手改的内容覆盖回去。
    const parsedEntries = parseGrokModelEntries(value);
    setModelEntries(parsedEntries.entries);
    setDefaultModelKey(parsedEntries.defaultKey);
    setUpstreamModel(parsed.upstreamModel ?? parsed.model);
    setBaseUrl(parsed.baseUrl);
    setApiKey(parsed.apiKey);
    setContextWindow(String(parsed.contextWindow));
    if (parsed.name) form.setValue("name", parsed.name);
  };

  const handleSubmit = async (values: ProviderFormData) => {
    const name = values.name.trim();

    // 官方条目：config 快照原样透传（新增时为空），不做自定义模型字段校验，
    // 也不重建 config —— 新增走 ensure seed，编辑只允许改名称/图标等元信息。
    if (category === "official") {
      await onSubmit({
        ...values,
        name,
        websiteUrl: values.websiteUrl?.trim() ?? "",
        notes: values.notes?.trim() ?? "",
        settingsConfig: JSON.stringify({ config: rawConfig }),
        presetId: selectedPresetId ?? undefined,
        presetCategory: "official",
        isPartner: false,
        meta: initialData?.meta,
      });
      return;
    }

    const envKey = parseGrokBuildConfig(rawConfig).envKey?.trim();
    if (
      !name ||
      !baseUrl.trim() ||
      (!apiKey.trim() && !envKey) ||
      !profile.trim()
    ) {
      toast.error(
        t("providerForm.requiredFields", {
          defaultValue: "请填写供应商名称、API 地址、API Key 和模型",
        }),
      );
      return;
    }

    const entriesResult = validateGrokModelEntries(
      { entries: modelEntries, defaultKey: defaultModelKey, modelsExtra: {} },
      { baseUrl, apiKey },
    );
    if (entriesResult) {
      toast.error(entriesResult);
      return;
    }
    const finalConfig = updateGrokModelEntries(
      rawConfig,
      {
        entries: modelEntries,
        defaultKey: defaultModelKey,
        modelsExtra: parseGrokModelEntries(rawConfig).modelsExtra,
      },
      { baseUrl, apiKey, apiBackend: GROK_BUILD_DEFAULT_API_BACKEND },
    );
    const configError = validateGrokBuildConfig(finalConfig);
    if (configError) {
      toast.error(
        t("grokBuild.invalidToml", {
          error: configError,
          defaultValue: `config.toml 格式错误: ${configError}`,
        }),
      );
      return;
    }

    const requestOverrides = buildLocalProxyRequestOverrides(
      headersOverride,
      bodyOverride,
    );
    if (requestOverrides.error) {
      toast.error(requestOverrides.error);
      return;
    }

    const customEndpoints = Object.fromEntries(
      draftCustomEndpoints.map((url) => [
        url,
        { url, addedAt: Date.now(), lastUsed: undefined },
      ]),
    );
    const parsedMaxOutputTokens = Number.parseInt(maxOutputTokens, 10);
    const initialMeta = { ...(initialData?.meta ?? {}) };
    delete initialMeta.custom_endpoints;
    const meta: ProviderMeta = {
      ...initialMeta,
      apiFormat,
      apiKeyField: anthropicAuthField,
      isFullUrl,
      endpointAutoSelect,
      isPartner,
      partnerPromotionKey,
      impersonateClaudeCode,
      promptCacheRouting,
      codexChatReasoning,
      customUserAgent: customUserAgent.trim() || undefined,
      localProxyRequestOverrides: requestOverrides.overrides,
      maxOutputTokens:
        Number.isInteger(parsedMaxOutputTokens) && parsedMaxOutputTokens > 0
          ? parsedMaxOutputTokens
          : undefined,
    };
    if (!providerId && Object.keys(customEndpoints).length > 0) {
      meta.custom_endpoints = customEndpoints;
    }
    const payload: ProviderFormValues = {
      ...values,
      name,
      websiteUrl: values.websiteUrl?.trim() ?? "",
      notes: values.notes?.trim() ?? "",
      settingsConfig: JSON.stringify({ config: finalConfig }),
      presetId: selectedPresetId ?? undefined,
      presetCategory: category ?? "custom",
      isPartner,
      meta,
    };

    await onSubmit(payload);
  };

  const rawConfigError = validateGrokBuildConfig(rawConfig);

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
            presetEntries={grokPresetEntries}
            presetCategoryLabels={presetCategoryLabels}
            onPresetChange={handlePresetChange}
            category={category}
          />
        )}

        <BasicFormFields form={form} />

        {category !== "official" && (
          <>
            <CodexFormFields
              appId="grokbuild"
              providerId={providerId}
              codexApiKey={apiKey}
              onApiKeyChange={(value) => {
                setApiKey(value);
                rebuildConfig({ apiKey: value });
              }}
              category={category}
              shouldShowApiKeyLink={Boolean(websiteUrl)}
              websiteUrl={websiteUrl}
              isPartner={isPartner}
              partnerPromotionKey={partnerPromotionKey}
              shouldShowSpeedTest
              codexBaseUrl={baseUrl}
              onBaseUrlChange={(value) => {
                setBaseUrl(value);
                rebuildConfig({ baseUrl: value });
              }}
              isFullUrl={isFullUrl}
              onFullUrlChange={setIsFullUrl}
              isEndpointModalOpen={isEndpointModalOpen}
              onEndpointModalToggle={setIsEndpointModalOpen}
              onCustomEndpointsChange={setDraftCustomEndpoints}
              autoSelect={endpointAutoSelect}
              onAutoSelectChange={setEndpointAutoSelect}
              codexModel={upstreamModel}
              apiFormat={apiFormat}
              onApiFormatChange={(value) => {
                setApiFormat(value);
              }}
              anthropicAuthField={anthropicAuthField}
              onAnthropicAuthFieldChange={setAnthropicAuthField}
              impersonateClaudeCode={impersonateClaudeCode}
              onImpersonateClaudeCodeChange={setImpersonateClaudeCode}
              maxOutputTokens={maxOutputTokens}
              onMaxOutputTokensChange={setMaxOutputTokens}
              codexChatReasoning={codexChatReasoning}
              onCodexChatReasoningChange={setCodexChatReasoning}
              promptCacheRouting={promptCacheRouting}
              onPromptCacheRoutingChange={setPromptCacheRouting}
              speedTestEndpoints={speedTestEndpoints}
              customUserAgent={customUserAgent}
              onCustomUserAgentChange={setCustomUserAgent}
              localProxyHeadersOverride={headersOverride}
              onLocalProxyHeadersOverrideChange={setHeadersOverride}
              localProxyBodyOverride={bodyOverride}
              onLocalProxyBodyOverrideChange={setBodyOverride}
            />
            <GrokModelEntriesEditor
              entries={modelEntries}
              defaultKey={defaultModelKey}
              baseUrl={baseUrl}
              apiKey={apiKey}
              onChange={({ entries: nextEntries, defaultKey: nextDefaultKey }) => {
                // 条目与默认值一次写入、一次重建：
                // 分两次会在同一 tick 内用旧的 entries 再写一遍，
                // 把已删除的行以幽灵条目形式写回 config.toml。
                setModelEntries(nextEntries);
                setDefaultModelKey(nextDefaultKey);
                rebuildConfig({
                  entries: nextEntries,
                  defaultKey: nextDefaultKey,
                });
              }}
            />


            <div className="space-y-2">
              <FormLabel htmlFor="grokbuild-config-toml">
                {t("grokBuild.rawConfig", { defaultValue: "config.toml" })}
              </FormLabel>
              <JsonEditor
                value={rawConfig}
                onChange={handleRawConfigChange}
                placeholder=""
                darkMode={isDarkMode}
                rows={3}
                showValidation={false}
                language="javascript"
              />
              {rawConfigError && (
                <p className="text-xs text-destructive">
                  {t("grokBuild.invalidToml", {
                    error: rawConfigError,
                    defaultValue: `Invalid config.toml: ${rawConfigError}`,
                  })}
                </p>
              )}
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
