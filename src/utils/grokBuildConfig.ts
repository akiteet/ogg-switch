import i18n from "@/i18n";
import { parse as parseToml, stringify as stringifyToml } from "smol-toml";

export const GROK_BUILD_DEFAULT_MODEL = "grok-4.6";
export const GROK_BUILD_DEFAULT_API_BACKEND = "responses";
export const GROK_BUILD_DEFAULT_CONTEXT_WINDOW = 500000;

export interface GrokBuildConfigValues {
  /** Client-visible profile selected by [models].default. */
  model: string;
  /** Real model sent to the upstream provider. */
  upstreamModel?: string;
  baseUrl: string;
  name: string;
  apiKey: string;
  envKey?: string;
  apiBackend: string;
  contextWindow: number;
}

const asRecord = (value: unknown): Record<string, unknown> | undefined =>
  value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;

const asString = (value: unknown, fallback = "") =>
  typeof value === "string" ? value : fallback;

export function parseGrokBuildConfig(
  configToml: string | undefined,
  fallbackName = "",
): GrokBuildConfigValues {
  const fallback: GrokBuildConfigValues = {
    model: GROK_BUILD_DEFAULT_MODEL,
    upstreamModel: GROK_BUILD_DEFAULT_MODEL,
    baseUrl: "",
    name: fallbackName,
    apiKey: "",
    apiBackend: GROK_BUILD_DEFAULT_API_BACKEND,
    contextWindow: GROK_BUILD_DEFAULT_CONTEXT_WINDOW,
  };

  if (!configToml?.trim()) return fallback;

  try {
    const root = asRecord(parseToml(configToml));
    const models = asRecord(root?.models);
    const defaultModel = asString(models?.default, GROK_BUILD_DEFAULT_MODEL);
    const modelTables = asRecord(root?.model);
    const selectedModel = asRecord(modelTables?.[defaultModel]);
    const rawContextWindow = selectedModel?.context_window;

    return {
      model: defaultModel,
      upstreamModel: asString(selectedModel?.model, defaultModel),
      baseUrl: asString(selectedModel?.base_url),
      // 供应商名只来自 fallbackName(即供应商数据),不借用模型条目的显示名,
      // 否则改模型字段会把供应商名一起改掉。
      name: fallbackName,
      apiKey: asString(selectedModel?.api_key),
      envKey: asString(selectedModel?.env_key),
      apiBackend: asString(
        selectedModel?.api_backend,
        GROK_BUILD_DEFAULT_API_BACKEND,
      ),
      contextWindow:
        typeof rawContextWindow === "number" &&
        Number.isInteger(rawContextWindow) &&
        rawContextWindow > 0
          ? rawContextWindow
          : GROK_BUILD_DEFAULT_CONTEXT_WINDOW,
    };
  } catch {
    return fallback;
  }
}

export function buildGrokBuildConfig(values: GrokBuildConfigValues): string {
  return updateGrokBuildConfig(undefined, values);
}

export function updateGrokBuildConfig(
  configToml: string | undefined,
  values: GrokBuildConfigValues,
): string {
  const profile = values.model.trim() || GROK_BUILD_DEFAULT_MODEL;
  const upstreamModel = values.upstreamModel?.trim() || profile;
  let config: Record<string, unknown> = {};

  try {
    config = asRecord(configToml?.trim() ? parseToml(configToml) : {}) ?? {};
  } catch {
    config = {};
  }

  const existingModels = asRecord(config.models) ?? {};
  const previousProfile = asString(existingModels.default, profile);
  config.models = { ...existingModels, default: profile };

  const modelTables = asRecord(config.model) ?? {};
  const existingSelected =
    asRecord(modelTables[profile]) ??
    asRecord(modelTables[previousProfile]) ??
    {};
  const apiKey = values.apiKey.trim();
  const envKey =
    values.envKey?.trim() || asString(existingSelected.env_key).trim();
  const updatedSelected: Record<string, unknown> = {
    ...existingSelected,
    model: upstreamModel,
    base_url: values.baseUrl.trim(),
    // name 只是模型选择器的显示标签:留空时用条目键名(模型 id)兜底
    name: values.name.trim() || profile,
    api_backend: values.apiBackend.trim() || GROK_BUILD_DEFAULT_API_BACKEND,
    context_window:
      Number.isInteger(values.contextWindow) && values.contextWindow > 0
        ? values.contextWindow
        : GROK_BUILD_DEFAULT_CONTEXT_WINDOW,
  };
  if (apiKey) updatedSelected.api_key = apiKey;
  else delete updatedSelected.api_key;
  if (envKey) updatedSelected.env_key = envKey;
  else delete updatedSelected.env_key;

  config.model = {
    ...modelTables,
    [profile]: updatedSelected,
  };

  if (previousProfile !== profile && previousProfile in modelTables) {
    delete (config.model as Record<string, unknown>)[previousProfile];
  }

  return `${stringifyToml(config).trim()}\n`;
}

export function validateGrokBuildConfig(configToml: string): string | null {
  if (!configToml.trim()) return "config.toml must not be empty";
  try {
    const root = asRecord(parseToml(configToml));
    const models = asRecord(root?.models);
    const profile = asString(models?.default).trim();
    const selected = asRecord(asRecord(root?.model)?.[profile]);
    if (!profile || !selected) return "Missing [models] default model table";
    // name 是显示标签,允许缺省(渲染时回落为模型 id),不作为必填校验
    for (const field of ["model", "base_url", "api_backend"]) {
      if (!asString(selected[field]).trim()) return `Missing ${field}`;
    }
    if (
      !asString(selected.api_key).trim() &&
      !asString(selected.env_key).trim()
    ) {
      return "Missing api_key or env_key";
    }
    const contextWindow = selected.context_window;
    if (
      typeof contextWindow !== "number" ||
      !Number.isInteger(contextWindow) ||
      contextWindow <= 0
    ) {
      return "context_window must be a positive integer";
    }
    return null;
  } catch (error) {
    return error instanceof Error ? error.message : "Invalid TOML";
  }
}

export function extractGrokBuildBaseUrl(configToml: string): string {
  return parseGrokBuildConfig(configToml).baseUrl;
}

// ---------------------------------------------------------------------------
// 全量模型条目读写(grok-switch 新增)
//
// cc-switch 原版表单只读写 [models].default 指向的单个条目,当 config.toml
// 里存在多个 [model.*] 时,表单与文件不一致。以下函数让表单能够完整展示并
// 编辑全部条目,保证"表单看到什么,config.toml 就是什么"。
// ---------------------------------------------------------------------------

export interface GrokModelEntry {
  /** 模型 ID:同时作为 [model."<id>"] 的键名与该条目的 model 字段。 */
  id: string;
  /** 显示名(选择器标签),留空时等于 id。 */
  displayName: string;
  /** 上下文窗口(有效参数,写入 [model."<id>"].context_window)。 */
  contextWindow: number;
  /** reasoning_efforts 等额外字段原样保留。 */
  extra: Record<string, unknown>;
}

export interface GrokModelEntries {
  entries: GrokModelEntry[];
  /** [models].default 指向的条目键名。 */
  defaultKey: string;
  /** [models] 下的其它字段(default_reasoning_effort / web_search 等)。 */
  modelsExtra: Record<string, unknown>;
}

/** 解析 config.toml 里的全部 [model.*] 条目。解析失败返回空。 */
export function parseGrokModelEntries(
  configToml: string | undefined,
): GrokModelEntries {
  if (!configToml?.trim()) {
    return { entries: [], defaultKey: "", modelsExtra: {} };
  }
  let root: Record<string, unknown> | undefined;
  try {
    root = asRecord(parseToml(configToml) as unknown);
  } catch {
    return { entries: [], defaultKey: "", modelsExtra: {} };
  }
  const models = asRecord(root?.models);
  const defaultKey = asString(models?.default).trim();
  const modelsExtra: Record<string, unknown> = { ...(models ?? {}) };
  delete modelsExtra.default;
  const modelTables = asRecord(root?.model) ?? {};
  const entries: GrokModelEntry[] = [];
  for (const [key, value] of Object.entries(modelTables)) {
    const table = asRecord(value) ?? {};
    const extra: Record<string, unknown> = { ...table };
    for (const field of [
      "model",
      "name",
      "base_url",
      "api_key",
      "env_key",
      "api_backend",
      "context_window",
    ]) {
      delete extra[field];
    }
    const rawContextWindow = table.context_window;
    entries.push({
      id: key,
      displayName: asString(table.name, key) || key,
      contextWindow:
        typeof rawContextWindow === "number" &&
        Number.isInteger(rawContextWindow) &&
        rawContextWindow > 0
          ? rawContextWindow
          : GROK_BUILD_DEFAULT_CONTEXT_WINDOW,
      extra,
    });
  }
  return { entries, defaultKey, modelsExtra };
}

/** 用全量条目重写 config.toml 的模型段,非模型段([cli]/[ui]/[marketplace] 等)原样保留。 */
export function updateGrokModelEntries(
  configToml: string | undefined,
  data: GrokModelEntries,
  supplier: { baseUrl: string; apiKey: string; apiBackend?: string },
): string {
  let root: Record<string, unknown> = {};
  if (configToml?.trim()) {
    try {
      root = (asRecord(parseToml(configToml) as unknown) ?? {}) as Record<string, unknown>;
    } catch {
      root = {};
    }
  }
  const modelTables: Record<string, unknown> = {};
  for (const entry of data.entries) {
    const id = entry.id.trim();
    if (!id) continue;
    const table: Record<string, unknown> = { ...entry.extra };
    // 模型 ID 同时作为 key 名与上游 model 字段;显示名单独存 name。
    table.model = id;
    table.name = entry.displayName.trim() || id;
    table.base_url = supplier.baseUrl.trim();
    table.api_backend = (supplier.apiBackend ?? "").trim() || GROK_BUILD_DEFAULT_API_BACKEND;
    table.context_window =
      Number.isInteger(entry.contextWindow) && entry.contextWindow > 0
        ? entry.contextWindow
        : GROK_BUILD_DEFAULT_CONTEXT_WINDOW;
    if (supplier.apiKey.trim()) table.api_key = supplier.apiKey.trim();
    else delete table.api_key;
    modelTables[id] = table;
  }
  const keys = Object.keys(modelTables);
  const defaultKey = keys.includes(data.defaultKey) ? data.defaultKey : (keys[0] ?? "");
  root.models = { ...data.modelsExtra, default: defaultKey };
  root.model = modelTables;
  return `${stringifyToml(root).trim()}\n`;
}

/**
 * 把 `defaultKey` 校正为**确实存在于条目列表中**的值。
 *
 * `defaultKey` 是模型列表的第二份"真相",任何条目的增删改都可能让它指向
 * 已不存在的 ID。这里统一兜底:不在列表里就回落到首个条目(空列表则为空串),
 * 使 UI 永远恰有一行被标记为默认、保存校验也不会因漂移而失败。
 */
export function reconcileDefaultKey(
  entries: GrokModelEntry[],
  defaultKey: string,
): string {
  const trimmed = defaultKey.trim();
  if (trimmed && entries.some((entry) => entry.id.trim() === trimmed)) {
    return trimmed;
  }
  return entries[0]?.id.trim() ?? "";
}

/**
 * 返回删除 `removedIndex` 后应作为默认的条目 ID。
 *
 * 保留默认值(被删的不是默认行);被删的正是默认行时**顺延**到原位置上的
 * 下一行(即删除后同索引的那条),没有下一行则取上一行,都没有则空串。
 */
export function defaultKeyAfterRemoval(
  entries: GrokModelEntry[],
  removedIndex: number,
  defaultKey: string,
): string {
  const survivors = entries.filter((_, index) => index !== removedIndex);
  if (survivors.length === 0) return "";
  const current = reconcileDefaultKey(entries, defaultKey);
  const removedId = entries[removedIndex]?.id.trim() ?? "";
  if (current !== removedId) return current;
  const fallback = survivors[removedIndex] ?? survivors[removedIndex - 1];
  return (fallback ?? survivors[0]).id.trim();
}

/** 校验:供应商级 URL/Key 各一次,条目级校验 条目名/上游模型/上下文窗口。 */
export function validateGrokModelEntries(
  data: GrokModelEntries,
  supplier: { baseUrl: string; apiKey: string },
): string | null {
  if (!supplier.baseUrl.trim()) return i18n.t("grokBuild.validation.baseUrlRequired");
  if (!supplier.apiKey.trim()) return i18n.t("grokBuild.validation.apiKeyRequired");
  if (!data.entries.length) return i18n.t("grokBuild.validation.atLeastOneEntry");
  const seen = new Set<string>();
  for (const entry of data.entries) {
    const id = entry.id.trim();
    if (!id) return i18n.t("grokBuild.validation.modelIdRequired");
    if (seen.has(id)) return i18n.t("grokBuild.validation.modelIdDuplicate", { id });
    seen.add(id);
    if (!Number.isInteger(entry.contextWindow) || entry.contextWindow <= 0) {
      return i18n.t("grokBuild.validation.contextWindowPositive", { id });
    }
  }
  if (data.defaultKey && !data.entries.some((e) => e.id.trim() === data.defaultKey)) {
    return i18n.t("grokBuild.validation.defaultKeyNotInEntries", {
      key: data.defaultKey,
    });
  }
  return null;
}
