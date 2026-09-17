/**
 * OMP 模型列表行编辑器
 *
 * 替代原来的裸 JSON textarea：每行可编辑 模型ID / 显示名 / 上下文窗口 / 最大输出 /
 * 推理 / API 协议，并支持「获取模型列表」（调用通用 fetch_models_for_config 或
 * OMP 自身的 omp models）。
 */

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Plus, Trash2, Download, Loader2 } from "lucide-react";
import type { OmpModelInfo, OmpApiProtocol } from "@/types/omp";
import { ompApi } from "@/lib/api";
import { ModelDropdown } from "@/components/providers/forms/shared";

interface OmpModelListEditorProps {
  models: OmpModelInfo[];
  onModelsChange: (models: OmpModelInfo[]) => void;
  /** 供应商级 baseUrl / apiKey，用于「获取模型列表」的 HTTP 回退路径 */
  baseUrl?: string;
  apiKey?: string;
  /** true → Bearer；false/缺省 → X-Api-Key（与 omp models.yml authHeader 语义一致） */
  authHeader?: boolean;
  /** OMP 供应商 id：存在时优先走 OMP 原生目录（omp models --json，无需凭据） */
  providerId?: string;
}

/** 抓取结果（比通用 FetchedModel 宽：保留 OMP 目录自带的上下文/推理元数据） */
type FetchedOmpModel = {
  id: string;
  name?: string;
  ownedBy?: string | null;
  contextWindow?: number;
  maxTokens?: number;
  reasoning?: boolean;
};

const PROTOCOL_OPTIONS: { value: OmpApiProtocol | ""; label: string }[] = [
  { value: "", label: "继承供应商" },
  { value: "openai-completions", label: "OpenAI Completions" },
  { value: "openai-responses", label: "OpenAI Responses" },
  { value: "anthropic-messages", label: "Anthropic Messages" },
  { value: "google-generative-ai", label: "Google Generative AI" },
];

export function OmpModelListEditor({
  models,
  onModelsChange,
  baseUrl,
  apiKey,
  authHeader,
  providerId,
}: OmpModelListEditorProps) {
  const { t } = useTranslation();
  const [isFetching, setIsFetching] = useState(false);
  const [fetched, setFetched] = useState<FetchedOmpModel[]>([]);

  const update = (index: number, patch: Partial<OmpModelInfo>) => {
    onModelsChange(models.map((m, i) => (i === index ? { ...m, ...patch } : m)));
  };

  const addModel = () => {
    onModelsChange([
      ...models,
      {
        id: "",
        name: "",
        contextWindow: 0,
        maxTokens: 0,
        reasoning: false,
      },
    ]);
  };

  const removeModel = (index: number) => {
    onModelsChange(models.filter((_, i) => i !== index));
  };

  const handleFetch = async () => {
    const id = providerId?.trim();
    const canHttp = Boolean(baseUrl?.trim() && apiKey?.trim());
    if (!id && !canHttp) {
      toast.error(
        t("omp.fetchNeedCreds", { defaultValue: "请先填写 Base URL 和 API Key" }),
      );
      return;
    }
    setIsFetching(true);
    try {
      let result: FetchedOmpModel[] = [];
      let usedNative = false;
      // 路径①：OMP 原生目录（omp models --json）。无需密钥——models.yml 里的
      // apiKey 往往是 secret-bridge 命令而非明文，直接当密钥用必然 401。
      if (id) {
        try {
          const native = await ompApi.ompListModels(id);
          if (native.length > 0) {
            result = native.map((m) => ({
              id: m.id,
              name: m.name,
              contextWindow: m.contextWindow,
              maxTokens: m.maxTokens,
              reasoning: m.reasoning,
            }));
            usedNative = true;
          }
        } catch (nativeError) {
          console.warn(
            "[OmpModelListEditor] omp native list failed:",
            nativeError,
          );
        }
      }
      // 路径②：HTTP /v1/models 回退。走后端命令：密钥可能是 $ENV / secret-bridge
      // 形态，前端直接当明文发请求必 401（后端负责解析）。
      if (!usedNative && canHttp) {
        const http = await ompApi.ompFetchUpstreamModels(
          baseUrl!.trim(),
          apiKey!.trim(),
          authHeader,
        );
        result = http.map((m) => ({ id: m.id, name: m.name }));
      }
      setFetched(result);
      if (result.length === 0) {
        toast.info(t("providerForm.fetchModelsEmpty", { defaultValue: "未获取到模型" }));
      } else {
        toast.success(
          usedNative
            ? t("omp.fetchedFromOmp", {
                count: result.length,
                defaultValue: `从 OMP 模型目录获取到 ${result.length} 个模型`,
              })
            : t("providerForm.fetchModelsSuccess", {
                count: result.length,
                defaultValue: `获取到 ${result.length} 个模型`,
              }),
        );
      }
    } catch (error) {
      console.error("[OmpModelListEditor] fetch failed:", error);
      toast.error(
        t("providerForm.fetchModelsFailed", {
          defaultValue: "获取模型列表失败，请检查 Base URL 与 API Key",
        }),
      );
    } finally {
      setIsFetching(false);
    }
  };

  /** 把抓到的模型合并进列表（不覆盖已有同 id 项；保留 OMP 目录元数据） */
  const importFetched = () => {
    const existing = new Set(models.map((m) => m.id));
    const additions = fetched
      .filter((f) => f.id && !existing.has(f.id))
      .map<OmpModelInfo>((f) => ({
        id: f.id,
        name: f.name || f.id,
        contextWindow: f.contextWindow ?? 0,
        maxTokens: f.maxTokens ?? 0,
        reasoning: f.reasoning ?? false,
      }));
    if (additions.length === 0) {
      toast.info(t("omp.noNewModels", { defaultValue: "没有可导入的新模型" }));
      return;
    }
    onModelsChange([...models, ...additions]);
    toast.success(
      t("omp.importedModels", {
        count: additions.length,
        defaultValue: `已导入 ${additions.length} 个模型`,
      }),
    );
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <div>
          <h4 className="text-sm font-medium">
            {t("omp.models", { defaultValue: "模型列表" })}
          </h4>
          <p className="text-xs text-muted-foreground">
            {t("omp.modelsHint", { defaultValue: "为空时由 OMP 从上游自动发现" })}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {(providerId?.trim() || baseUrl) && (
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={handleFetch}
              disabled={isFetching}
            >
              {isFetching ? (
                <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
              ) : (
                <Download className="mr-1 h-3.5 w-3.5" />
              )}
              {t("providerForm.fetchModels", { defaultValue: "获取模型列表" })}
            </Button>
          )}
          <Button type="button" size="sm" variant="outline" onClick={addModel}>
            <Plus className="mr-1 h-3.5 w-3.5" />
            {t("omp.addModel", { defaultValue: "添加模型" })}
          </Button>
        </div>
      </div>

      {fetched.length > 0 && (
        <div className="flex items-center justify-between rounded-md border border-dashed p-2 text-xs">
          <span className="text-muted-foreground">
            {t("omp.fetchedCount", {
              count: fetched.length,
              defaultValue: `已获取 ${fetched.length} 个模型`,
            })}
          </span>
          <Button type="button" size="sm" variant="secondary" onClick={importFetched}>
            {t("omp.importAll", { defaultValue: "全部导入" })}
          </Button>
        </div>
      )}

      {models.length === 0 && (
        <p className="rounded-md border border-dashed p-4 text-center text-xs text-muted-foreground">
          {t("omp.noModels", { defaultValue: "还没有模型" })}
        </p>
      )}

      <div className="space-y-2">
        {models.map((model, index) => (
          <div
            key={index}
            className="space-y-2 rounded-lg border border-border-default p-3"
          >
            <div className="flex items-center gap-2">
              <div className="flex flex-1 gap-1">
                <Input
                  value={model.id}
                  onChange={(e) => update(index, { id: e.target.value })}
                  placeholder="model-id"
                  className="flex-1 font-mono text-sm"
                />
                {fetched.length > 0 && (
                  <ModelDropdown
                    models={fetched}
                    onSelect={(id) => update(index, { id })}
                  />
                )}
              </div>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="text-destructive"
                onClick={() => removeModel(index)}
              >
                <Trash2 className="h-3.5 w-3.5" />
              </Button>
            </div>
            <div className="grid grid-cols-2 gap-2">
              <Input
                value={model.name ?? ""}
                onChange={(e) => update(index, { name: e.target.value })}
                placeholder={t("omp.displayName", { defaultValue: "显示名（可选）" })}
                className="text-sm"
              />
              <Select
                value={model.api ?? ""}
                onValueChange={(v) =>
                  update(index, {
                    api: (v || undefined) as OmpApiProtocol | undefined,
                  })
                }
              >
                <SelectTrigger className="text-sm">
                  <SelectValue placeholder={t("omp.apiProtocol", { defaultValue: "API 协议" })} />
                </SelectTrigger>
                <SelectContent>
                  {PROTOCOL_OPTIONS.map((p) => (
                    <SelectItem key={p.value || "inherit"} value={p.value || "__inherit__"}>
                      {p.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid grid-cols-2 gap-2">
              <Input
                type="number"
                value={model.contextWindow || ""}
                onChange={(e) =>
                  update(index, {
                    contextWindow: Number(e.target.value.replace(/[^0-9]/g, "")) || 0,
                  })
                }
                placeholder={t("omp.contextWindow", { defaultValue: "上下文窗口" })}
                className="text-sm"
              />
              <Input
                type="number"
                value={model.maxTokens || ""}
                onChange={(e) =>
                  update(index, {
                    maxTokens: Number(e.target.value.replace(/[^0-9]/g, "")) || 0,
                  })
                }
                placeholder={t("omp.maxTokens", { defaultValue: "最大输出" })}
                className="text-sm"
              />
            </div>
            <label className="flex items-center gap-2 text-xs">
              <Checkbox
                checked={model.reasoning === true}
                onCheckedChange={(c) => update(index, { reasoning: !!c })}
              />
              {t("omp.reasoning", { defaultValue: "支持推理 (reasoning)" })}
              {model.reasoning && (
                <Badge variant="secondary" className="text-[10px]">
                  reasoning
                </Badge>
              )}
            </label>
          </div>
        ))}
      </div>
    </div>
  );
}
