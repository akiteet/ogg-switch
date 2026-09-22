import { useRef, useState } from "react";
import { Download, Loader2, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  ModelDropdown,
  SortableRows,
  SortableRow,
  RowDragHandle,
  reorderAligned,
} from "@/components/providers/forms/shared";
import { cn } from "@/lib/utils";
import { fetchModelsForConfig, type FetchedModel } from "@/lib/api/model-fetch";
import {
  defaultKeyAfterRemoval,
  reconcileDefaultKey,
  type GrokModelEntry,
} from "@/utils/grokBuildConfig";

interface GrokModelEntriesEditorProps {
  entries: GrokModelEntry[];
  defaultKey: string;
  /**
   * 条目与默认值**一次**发出。
   *
   * 此前分成 onEntriesChange / onDefaultChange 两个回调，删除默认行会在同一
   * tick 里触发两次重建，第二次读到的还是旧条目 → 已删除的行被写回配置
   * （幽灵条目）。合并成单次变更后不会再有这种中间态。
   */
  onChange: (next: { entries: GrokModelEntry[]; defaultKey: string }) => void;
  /** 供应商级 Base URL，用于「获取模型列表」。 */
  baseUrl: string;
  /** 供应商级 API Key，用于「获取模型列表」。 */
  apiKey: string;
}

/**
 * Grok Build 模型列表编辑器。
 *
 * 每行三件事:
 *   - 模型 ID:既作为 config.toml 里 [model."<id>"] 的键名,也是发给上游的 model;
 *     右侧的 chevron 下拉可从「获取模型列表」的结果中直接挑选。
 *   - 显示名:仅用于 grok CLI 的 /model 选择器展示,留空则等于模型 ID。
 *   - 上下文窗口。
 *
 * base_url / api_key 属于供应商级,在表单其他位置填写一次,写入时盖到每张模型表上。
 */
export function GrokModelEntriesEditor({
  entries,
  defaultKey,
  onChange,
  baseUrl,
  apiKey,
}: GrokModelEntriesEditorProps) {
  const { t } = useTranslation();
  const [isFetching, setIsFetching] = useState(false);
  const [fetchedModels, setFetchedModels] = useState<FetchedModel[]>([]);

  // 行的 React key 必须与可编辑字段(模型 ID)解耦:用稳定 uuid。
  // 否则每按一次 backspace 改变 id,React 就重建该行输入框 → 焦点丢失。
  const rowKeysRef = useRef<string[]>([]);
  const rowKeys = (() => {
    const keys = rowKeysRef.current;
    while (keys.length < entries.length) keys.push(crypto.randomUUID());
    if (keys.length > entries.length) keys.length = entries.length;
    return keys;
  })();

  // 单一真相:UI 显示的默认行恒取自条目数据本身。
  // 上游 state 若漂移(例如默认行的 ID 刚被改过)也不会出现"没有任何一行被选中"。
  const effectiveDefaultKey = reconcileDefaultKey(entries, defaultKey);

  const updateEntry = (index: number, patch: Partial<GrokModelEntry>) => {
    const nextEntries = entries.map((entry, i) =>
      i === index ? { ...entry, ...patch } : entry,
    );
    // 改的若是默认行的 ID,默认值要跟着走,否则它会指向不存在的条目。
    const editingDefaultRow = (entries[index]?.id.trim() ?? "") === effectiveDefaultKey;
    const nextDefaultKey = editingDefaultRow
      ? (nextEntries[index]?.id ?? "")
      : effectiveDefaultKey;
    onChange({ entries: nextEntries, defaultKey: nextDefaultKey });
  };

  const addEntry = () => {
    const base = "grok-4.6";
    let id = base;
    let n = 2;
    while (entries.some((e) => e.id === id)) id = `${base}-${n++}`;
    const nextEntries = [
      ...entries,
      { id, displayName: id, contextWindow: 500000, extra: {} },
    ];
    onChange({
      entries: nextEntries,
      defaultKey: effectiveDefaultKey || id,
    });
  };

  const removeEntry = (index: number) => {
    const nextEntries = entries.filter((_, i) => i !== index);
    rowKeysRef.current = rowKeys.filter((_, i) => i !== index);
    onChange({
      entries: nextEntries,
      // 删的是默认行 → 顺延到相邻行;否则保持原默认值。
      defaultKey: defaultKeyAfterRemoval(entries, index, effectiveDefaultKey),
    });
  };

  // 拖动重排：行 id（uuid）驱动。默认模型以条目 id 标识、不绑定行位置，
  // 行序变化不影响它是哪一行，原样带回即可（reconcileDefaultKey 会校验存在性）。
  const handleReorder = (activeRowId: string, overRowId: string) => {
    const nextEntries = reorderAligned(rowKeys, entries, activeRowId, overRowId);
    onChange({ entries: nextEntries, defaultKey: effectiveDefaultKey });
  };

  // 只拉取列表并展示在「模型 ID」右侧的下拉里,不自动改动条目。
  const handleFetchModels = async () => {
    if (!baseUrl.trim() || !apiKey.trim()) {
      toast.error(
        t("providerForm.fetchModelsNeedCreds", {
          defaultValue: "请先填写 Base URL 和 API Key",
        }),
      );
      return;
    }
    setIsFetching(true);
    try {
      const models = await fetchModelsForConfig(baseUrl, apiKey);
      setFetchedModels(models);
      if (models.length === 0) {
        toast.info(t("providerForm.fetchModelsEmpty"));
      } else {
        toast.success(
          t("providerForm.fetchModelsSuccess", { count: models.length }),
        );
      }
    } catch (error) {
      console.warn("[ModelFetch] Failed:", error);
      toast.error(
        t("providerForm.fetchModelsFailed", {
          defaultValue: "获取模型列表失败,请检查 Base URL 与 API Key",
        }),
      );
    } finally {
      setIsFetching(false);
    }
  };

  return (
    <section className="space-y-3">
      <header className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-medium">
            {t("grokBuild.models.title", { defaultValue: "模型配置" })}
          </h3>
        </div>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={handleFetchModels}
            disabled={isFetching}
          >
            {isFetching ? (
              <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
            ) : (
              <Download className="mr-1 h-3.5 w-3.5" />
            )}
            {t("providerForm.fetchModels", { defaultValue: "获取模型列表" })}
          </Button>
          <Button type="button" size="sm" variant="outline" onClick={addEntry}>
            <Plus className="mr-1 h-3.5 w-3.5" />
            {t("grokBuild.models.add", { defaultValue: "添加模型" })}
          </Button>
        </div>
      </header>

      {entries.length === 0 && (
        <p className="rounded-md border border-dashed border-border-default p-4 text-center text-xs text-muted-foreground">
          {t("grokBuild.models.empty", { defaultValue: "还没有模型" })}
        </p>
      )}

      <SortableRows rowIds={rowKeys} onReorder={handleReorder}>
        <div className="space-y-3">
          {entries.map((entry, index) => (
            <SortableRow key={rowKeys[index]} rowId={rowKeys[index]!}>
              {({ attributes, listeners, isDragging }) => (
                <div
                  className={cn(
                    "space-y-3 rounded-lg border p-4",
                    entry.id === effectiveDefaultKey
                      ? "border-primary/50 bg-primary/5"
                      : "border-border-default",
                    isDragging && "border-primary/60 bg-background shadow-md",
                  )}
                >
                  <div className="flex items-center gap-3">
                    <RowDragHandle
                      attributes={attributes}
                      listeners={listeners}
                      isDragging={isDragging}
                    />
                    <input
                      type="radio"
                      name="grok-default-model"
                      className="size-4 accent-primary"
                      checked={entry.id === effectiveDefaultKey}
                      onChange={() =>
                        onChange({ entries, defaultKey: entry.id })
                      }
                      aria-label={t("grokBuild.models.setDefault", {
                        defaultValue: "设为默认",
                      })}
                    />
                    <span className="text-xs text-muted-foreground">
                      {entry.id === effectiveDefaultKey
                        ? t("grokBuild.models.isDefault", { defaultValue: "默认模型" })
                        : t("grokBuild.models.setDefaultHint", {
                            defaultValue: "设为默认",
                          })}
                    </span>
                    <div className="ml-auto">
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        className="text-destructive"
                        disabled={entries.length <= 1}
                        onClick={() => removeEntry(index)}
                      >
                        <Trash2 className="mr-1 h-3.5 w-3.5" />
                        {t("common.delete", { defaultValue: "删除" })}
                      </Button>
                    </div>
                  </div>

                  <div className="grid grid-cols-3 gap-3">
                    <div className="space-y-1">
                      <Label className="text-xs">
                        {t("grokBuild.models.id", { defaultValue: "模型 ID" })}
                      </Label>
                      <div className="flex gap-1">
                        <Input
                          value={entry.id}
                          onChange={(e) => updateEntry(index, { id: e.target.value })}
                          placeholder="grok-4.6"
                          className="flex-1"
                        />
                        {fetchedModels.length > 0 && (
                          <ModelDropdown
                            models={fetchedModels}
                            onSelect={(id) => updateEntry(index, { id })}
                          />
                        )}
                      </div>
                    </div>
                    <div className="space-y-1">
                      <Label className="text-xs">
                        {t("grokBuild.models.displayName", { defaultValue: "显示名" })}
                      </Label>
                      <Input
                        value={entry.displayName}
                        onChange={(e) =>
                          updateEntry(index, { displayName: e.target.value })
                        }
                        placeholder={entry.id}
                      />
                    </div>
                    <div className="space-y-1">
                      <Label className="text-xs">
                        {t("grokBuild.models.contextWindow", { defaultValue: "上下文窗口" })}
                      </Label>
                      <Input
                        type="number"
                        min={1}
                        step={1}
                        value={String(entry.contextWindow)}
                        onChange={(e) =>
                          updateEntry(index, {
                            contextWindow:
                              Number(e.target.value.replace(/[^0-9]/g, "")) || 0,
                          })
                        }
                        placeholder="500000"
                      />
                    </div>
                  </div>
                </div>
              )}
            </SortableRow>
          ))}
        </div>
      </SortableRows>
    </section>
  );
}
