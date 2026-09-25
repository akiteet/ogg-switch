import { useEffect, useMemo, useState } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Form } from "@/components/ui/form";
import { providerSchema, type ProviderFormData } from "@/lib/schemas/provider";
import type { ProviderCategory } from "@/types";
import type { ProviderFormProps } from "./ProviderForm";
import { BasicFormFields } from "./BasicFormFields";
import {
  ProviderPresetSelector,
  type AnyPreset,
  type PresetEntry,
} from "./ProviderPresetSelector";
import { OmpOAuthFormFields } from "./OmpOAuthFormFields";
import { OmpApiKeyFormFields } from "./OmpApiKeyFormFields";
import { ALL_OMP_PRESETS } from "@/config/ompProviderPresets";
import type {
  OmpProviderConfig,
  OmpApiProtocol,
  OmpModelInfo,
  OmpProviderPreset,
} from "@/types/omp";
import { validateOmpConfig } from "@/utils/ompConfig";
import { resolveProviderIcon } from "@/utils/providerIcon";
import { ompApi } from "@/lib/api";

type OmpProviderFormProps = Omit<ProviderFormProps, "appId">;

/**
 * OMP 的供应商分组（预设选择器三组 + 自定义入口），category 状态沿用通用
 * ProviderCategory：
 * - official：OAuth 登录（omp 内置，凭据在 omp 凭据库，无需 baseUrl / apiKey）
 * - aggregator：API Key（omp 内置：官方 API、聚合站、中转站、本地推理）
 * - third_party：常见供应商（OGG 增补预设，不在 omp 官方 providers.md 清单内，
 *   如腾讯混元、豆包、one-api/new-api 模板）——落库为 `common`
 * - custom：自定义（选择器「自定义」入口）——落库为 `custom`
 */
const OMP_OAUTH_CATEGORY = "official";
const OMP_API_KEY_CATEGORY = "aggregator";
const OMP_COMMON_CATEGORY = "third_party";

/**
 * loopback 地址判定（与后端 `infer_type` 同一口径）：本地推理服务不需要密钥，
 * 无密钥保存时按 local 落库（校验也只要求 baseUrl）。
 */
function isLoopbackBaseUrl(url: string): boolean {
  const value = url.trim().toLowerCase();
  if (!value) return false;
  return (
    value.includes("localhost") ||
    value.includes("127.0.0.1") ||
    value.includes("[::1]") ||
    value.includes("://::1")
  );
}

const OMP_API_PROTOCOLS: readonly string[] = [
  "openai-completions",
  "openai-responses",
  "anthropic-messages",
  "google-generative-ai",
];

function isOmpApiProtocol(value: unknown): value is OmpApiProtocol {
  return typeof value === "string" && OMP_API_PROTOCOLS.includes(value);
}

/**
 * 非 OAuth 条目的字段一律「存在即回填」。models.yml 不存 type，后端按 baseUrl
 * 猜类型（loopback → local）；曾按 type==="api-key" 回填，导致本机中转站
 * （http://127.0.0.1:…）编辑时 apiKey / headers / authHeader 全部显示为空，
 * 保存还会清掉 headers、把 authHeader 改回 true。
 */
function storedNonOauthFields(config: OmpProviderConfig | null) {
  const record = (config && config.type !== "oauth"
    ? config
    : null) as unknown as Record<string, unknown> | null;
  const str = (key: string): string =>
    typeof record?.[key] === "string" ? (record[key] as string) : "";
  const headers = record?.headers;
  return {
    baseUrl: str("baseUrl"),
    apiKey: str("apiKey"),
    headers:
      headers && typeof headers === "object"
        ? (headers as Record<string, string>)
        : {},
    authHeader:
      typeof record?.authHeader === "boolean"
        ? (record.authHeader as boolean)
        : true,
  };
}

function parseStoredConfig(
  settingsConfig: Record<string, unknown> | undefined,
): OmpProviderConfig | null {
  if (!settingsConfig) return null;
  const raw = settingsConfig.config;
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
  if (settingsConfig.type && settingsConfig.id) {
    return settingsConfig as unknown as OmpProviderConfig;
  }
  return null;
}

/**
 * 从预设选择器的复合条目 id（`${type}:${id}`）解析预设。
 * API-key 预设与 OAuth 预设可能同名（anthropic/openai/google/xai），
 * 裸 id 查找会命中错误分组，必须带 type 联合匹配。
 */
function findPresetByEntryId(
  entryId: string | null | undefined,
): OmpProviderPreset | undefined {
  if (!entryId || entryId === "custom") return undefined;
  const sep = entryId.indexOf(":");
  if (sep > 0) {
    const type = entryId.slice(0, sep);
    const id = entryId.slice(sep + 1);
    return ALL_OMP_PRESETS.find((p) => p.type === type && p.id === id);
  }
  return ALL_OMP_PRESETS.find((p) => p.id === entryId);
}

export function OmpProviderForm({
  providerId,
  submitLabel,
  onSubmit,
  onCancel,
  onSubmittingChange,
  initialData,
  showButtons = true,
}: OmpProviderFormProps) {
  const { t } = useTranslation();

  const initialConfig = useMemo(
    () => parseStoredConfig(initialData?.settingsConfig),
    [initialData?.settingsConfig],
  );

  const [selectedPresetId, setSelectedPresetId] = useState<string | null>(
    initialData ? null : "custom",
  );
  const [category, setCategory] = useState<ProviderCategory>(
    initialData?.category ?? "custom",
  );
  // 认证方式只留两种（type/category 均不写 models.yml，归并无副作用）：
  // OAuth / API 密钥。旧的 gateway / local 在此一律按 api-key 呈现与保存。
  const [providerType, setProviderType] = useState<"oauth" | "api-key">(
    initialConfig?.type === "oauth" ? "oauth" : "api-key",
  );
  const [oauthProviderId, setOauthProviderId] = useState(
    initialConfig?.type === "oauth" ? initialConfig.oauthProviderId : "",
  );
  const [isLoggedIn, setIsLoggedIn] = useState(false);
  const storedFields = useMemo(
    () => storedNonOauthFields(initialConfig),
    [initialConfig],
  );
  const [baseUrl, setBaseUrl] = useState(storedFields.baseUrl);
  const [apiKey, setApiKey] = useState(storedFields.apiKey);
  const [apiProtocol, setApiProtocol] = useState<OmpApiProtocol>(
    isOmpApiProtocol(initialConfig?.api)
      ? initialConfig.api
      : "openai-completions",
  );
  const [headers, setHeaders] = useState<Record<string, string>>(
    storedFields.headers,
  );
  const [authHeader, setAuthHeader] = useState(storedFields.authHeader);
  const [models, setModels] = useState<OmpModelInfo[]>(
    initialConfig?.models ?? [],
  );

  const form = useForm<ProviderFormData>({
    resolver: zodResolver(providerSchema),
    defaultValues: {
      name: initialData?.name ?? "",
      websiteUrl: initialData?.websiteUrl ?? "",
      notes: initialData?.notes ?? "",
      settingsConfig: JSON.stringify({ config: initialConfig ?? {} }),
      icon:
        resolveProviderIcon("omp", initialData?.icon, initialData?.iconColor) ??
        "",
      iconColor: initialData?.iconColor ?? "",
    },
    mode: "onSubmit",
  });
  const { isSubmitting } = form.formState;

  useEffect(() => {
    onSubmittingChange?.(isSubmitting);
  }, [isSubmitting, onSubmittingChange]);

  // 预设选择器的条目 id 用 `${type}:${id}` 复合键：API-key 预设 id 已与内置目录对齐，
  // 会与 OAuth 预设同名（如 oauth:anthropic / api-key:anthropic），裸 id 会导致
  // React key 冲突与 findPresetById 命中错误分组
  const presetEntries: PresetEntry[] = useMemo(
    () =>
      ALL_OMP_PRESETS.map((preset) => ({
        id: `${preset.type}:${preset.id}`,
        preset: {
          name: preset.name,
          websiteUrl: preset.websiteUrl,
          settingsConfig: {},
          category:
            preset.type === "oauth"
              ? OMP_OAUTH_CATEGORY
              : preset.tier === "common"
                ? OMP_COMMON_CATEGORY
                : OMP_API_KEY_CATEGORY,
          icon: preset.icon,
        } as AnyPreset,
      })),
    [],
  );

  // 当前选中的预设（「自定义」或编辑态为 undefined）
  const selectedPreset = useMemo(
    () => findPresetByEntryId(selectedPresetId),
    [selectedPresetId],
  );

  // 预设分组与悬浮标签共用同一份文案：OAuth / API Key（内置）/ 常见供应商
  const presetGroups = useMemo(
    () => [
      {
        category: OMP_OAUTH_CATEGORY,
        label: t("omp.groupOAuth", { defaultValue: "OAuth 登录（omp 内置）" }),
      },
      {
        category: OMP_API_KEY_CATEGORY,
        label: t("omp.groupApiKey", { defaultValue: "API Key（omp 内置）" }),
      },
      {
        category: OMP_COMMON_CATEGORY,
        label: t("omp.groupCommon", { defaultValue: "常见供应商" }),
      },
    ],
    [t],
  );

  const presetCategoryLabels = useMemo(
    () => Object.fromEntries(presetGroups.map((g) => [g.category, g.label])),
    [presetGroups],
  );

  // 「获取模型列表」的 OMP 原生目录过滤 id：编辑态用现有 id；新建态用选中预设的
  // 真实 id（复合键解包）或按名称推导（与 buildProviderConfig 的 slug 规则一致）；
  // 推导不出则仅走 HTTP。
  const watchName = form.watch("name");
  const fetchProviderId =
    providerId ||
    selectedPreset?.id ||
    watchName
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "") ||
    undefined;

  const handlePresetChange = (presetId: string) => {
    setSelectedPresetId(presetId);
    if (presetId === "custom") {
      setProviderType("api-key");
      setCategory("custom");
      return;
    }
    const preset = findPresetByEntryId(presetId);
    if (!preset) return;
    form.setValue("name", preset.name);
    form.setValue("websiteUrl", preset.websiteUrl);
    // 预设品牌图标随预设带入（持久化到 OGG meta store，见 omp.rs）
    form.setValue("icon", preset.icon ?? "");
    // 只有两种认证：OAuth（omp 内置登录）与 API Key（内置 / 常见供应商 / 本地）
    setCategory(
      preset.type === "oauth"
        ? OMP_OAUTH_CATEGORY
        : preset.tier === "common"
          ? OMP_COMMON_CATEGORY
          : OMP_API_KEY_CATEGORY,
    );
    setProviderType(preset.type === "oauth" ? "oauth" : "api-key");
    if (preset.type === "oauth" && preset.oauthProviderId) {
      setOauthProviderId(preset.oauthProviderId);
    }
    if (preset.defaultApi) setApiProtocol(preset.defaultApi);
    if (preset.defaultBaseUrl) setBaseUrl(preset.defaultBaseUrl);
    // API Key 默认留空（对齐 cc-switch）：envKeyName 仅作为帮助信息，不预填占位值。
    // 本地推理（local）同样清空——它不需要密钥，残留旧值会让条目被存成 api-key。
    if (
      preset.type === "api-key" ||
      preset.type === "gateway" ||
      preset.type === "local"
    ) {
      setApiKey("");
    }
    // OAuth 预设不 seed 模型清单：模型由 omp 从上游自动发现
    //（实测：OAuth 登录的官方供应商不需要专门配置模型）。
    if (preset.type !== "oauth" && preset.defaultModels) {
      setModels(preset.defaultModels);
    }
  };

  const buildProviderConfig = (): OmpProviderConfig => {
    // id 保真是编辑语义的根基：编辑态必须沿用现有 id（否则保存会因键变化
    // 追加新条目而非更新），只有新建态才从名称推导。
    // 该 id 同时是 models.yml 的 provider key（omp /model 与角色 selector
    // 显示的就是它），因此纯中文等 slug 化为空的名称回落原始名称而不是
    // UUID——否则 omp 里会显示一串不可读的随机 id（历史 bug）。
    const rawName = (form.getValues("name") || "custom").trim();
    const slug = rawName
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "");
    const id = providerId ?? (slug || rawName.replace(/\//g, "-") || "custom");
    const name = form.getValues("name");
    const websiteUrl = form.getValues("websiteUrl") ?? "";
    const description = form.getValues("notes") ?? "";
    // icon 属 OGG UI 数据：随配置一起提交，后端落 omp_provider_meta.json
    const icon = form.getValues("icon") || undefined;
    // 模型显示名允许留空（表单 placeholder 写的是"显示名（可选）"），但 omp 的
    // schema 要求 name 非空——留空会写盘报 "must be name a non-empty string"。
    // 统一在提交前用 id 兜底，保证任何来源的模型条目都带有效 name。
    const normalizedModels = models.map((m) => ({
      ...m,
      name: m.name.trim() || m.id.trim(),
    }));

    switch (providerType) {
      case "oauth":
        return {
          id,
          name,
          type: "oauth",
          category: "subscription",
          description,
          websiteUrl,
          icon,
          models: normalizedModels,
          oauthProviderId: oauthProviderId || id,
          api: apiProtocol,
        };
      case "api-key": {
        // 无密钥 + loopback = 本地推理（Ollama / LM Studio 等预设
        // requiresApiKey:false）：按 local 保存，否则会被「api-key 必须有密钥」
        // 的校验拦死，这类供应商根本存不下去。
        if (!apiKey.trim() && isLoopbackBaseUrl(baseUrl)) {
          return {
            id,
            name,
            type: "local",
            category: "local",
            description,
            websiteUrl,
            icon,
            models: normalizedModels,
            baseUrl,
            api: apiProtocol,
            headers,
            authHeader,
          };
        }
        // category 状态承载分层：常见供应商预设 → common，自定义入口 → custom，
        // 其余（内置 API Key 预设 / 无预设来源）→ api。编辑态该状态来自既有
        // 条目的通用 category 映射，保存后原样保真。
        const ompCategory =
          category === OMP_COMMON_CATEGORY
            ? "common"
            : category === "custom"
              ? "custom"
              : "api";
        return {
          id,
          name,
          type: "api-key",
          category: ompCategory,
          description,
          websiteUrl,
          icon,
          models: normalizedModels,
          baseUrl,
          apiKey,
          api: apiProtocol,
          headers,
          authHeader,
        };
      }
    }
  };

  const handleFormSubmit = form.handleSubmit(async (data) => {
    try {
      const providerConfig = buildProviderConfig();
      const errors = validateOmpConfig({
        version: 1,
        providers: [providerConfig],
        roles: [],
      });
      if (errors.length > 0) {
        toast.error(
          errors[0]?.message ??
            t("providerForm.validationError", { defaultValue: "配置验证失败" }),
        );
        return;
      }
      await ompApi.saveOmpProvider(providerConfig);
      await onSubmit({
        ...data,
        name: providerConfig.name,
        websiteUrl: providerConfig.websiteUrl ?? "",
        settingsConfig: JSON.stringify({ config: providerConfig }),
        presetId: selectedPresetId ?? undefined,
        presetCategory: category,
      });
    } catch (error) {
      console.error("[OmpProviderForm] Submit error:", error);
      toast.error(
        t("providerForm.saveError", { defaultValue: "保存失败，请检查配置" }),
      );
    }
  });

  // 认证方式由身份决定：OAuth 预设或编辑既有 OAuth 供应商 → OAuth；
  // 其余（API Key 预设、自定义新建）只呈现 API Key——omp 可登录的 OAuth
  // provider 由其凭据库目录固定，不存在"自定义 OAuth"。
  const isOauthForm = selectedPreset
    ? selectedPreset.type === "oauth"
    : initialConfig?.type === "oauth";

  const oauthFields = (
    <OmpOAuthFormFields
      oauthProviderId={oauthProviderId}
      apiProtocol={apiProtocol}
      onApiProtocolChange={setApiProtocol}
      isLoggedIn={isLoggedIn}
      onLoginStateChange={setIsLoggedIn}
    />
  );

  const apiKeyFields = (
    <OmpApiKeyFormFields
      baseUrl={baseUrl}
      onBaseUrlChange={setBaseUrl}
      apiKey={apiKey}
      onApiKeyChange={setApiKey}
      apiProtocol={apiProtocol}
      onApiProtocolChange={setApiProtocol}
      headers={headers}
      onHeadersChange={setHeaders}
      authHeader={authHeader}
      onAuthHeaderChange={setAuthHeader}
      models={models}
      onModelsChange={setModels}
      providerId={fetchProviderId}
      apiKeyLinkUrl={
        selectedPreset?.docsUrl ??
        selectedPreset?.websiteUrl ??
        (providerId ? initialData?.websiteUrl : undefined)
      }
    />
  );

  return (
    <Form {...form}>
      <form
        id="provider-form"
        onSubmit={handleFormSubmit}
        className="space-y-6"
      >
        {!initialData && (
          <ProviderPresetSelector
            selectedPresetId={selectedPresetId}
            presetEntries={presetEntries}
            presetCategoryLabels={presetCategoryLabels}
            onPresetChange={handlePresetChange}
            category={category}
            groupHeaders={presetGroups}
          />
        )}

        <BasicFormFields form={form} />

        <div className="space-y-4">
          {isOauthForm ? oauthFields : apiKeyFields}
        </div>

        {showButtons && (
          <div className="flex justify-end gap-3">
            <Button
              type="button"
              variant="outline"
              onClick={onCancel}
              disabled={isSubmitting}
            >
              {t("common.cancel", { defaultValue: "取消" })}
            </Button>
            <Button type="submit" disabled={isSubmitting}>
              {isSubmitting
                ? t("common.saving", { defaultValue: "保存中..." })
                : (submitLabel ?? t("common.save", { defaultValue: "保存" }))}
            </Button>
          </div>
        )}
      </form>
    </Form>
  );
}
