import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, ChevronDown, ChevronUp, X, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import type { EnvConflict } from "@/types/env";
import { deleteEnvVars, envVarsInUse } from "@/lib/api/env";
import { toast } from "sonner";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

interface EnvWarningBannerProps {
  conflicts: EnvConflict[];
  onDismiss: () => void;
  onDeleted: () => void;
}

/**
 * OGG Switch 自己写入并托管的持久环境变量（与后端 env_checker 的排除集一致）。
 * 这些键出现在注册表 / shell rc 里是正常工作状态（agy 等工具靠它们认证），
 * 不是冲突——绝不能引导用户删除，否则等于删掉供应商凭据。
 */
const MANAGED_ENV_KEYS = new Set(["GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL"]);

function isManagedEnvKey(name: string): boolean {
  return MANAGED_ENV_KEYS.has(name.toUpperCase());
}

export function EnvWarningBanner({
  conflicts,
  onDismiss,
  onDeleted,
}: EnvWarningBannerProps) {
  const { t } = useTranslation();
  const [isExpanded, setIsExpanded] = useState(false);
  const [selectedConflicts, setSelectedConflicts] = useState<Set<string>>(
    new Set(),
  );
  const [isDeleting, setIsDeleting] = useState(false);
  const [showConfirmDialog, setShowConfirmDialog] = useState(false);
  // 待删变量是否被 OGG 里配置的供应商引用（打开确认框时查询）
  const [usage, setUsage] = useState<Map<string, string[]> | null>(null);

  // 双保险：受管键不该出现在冲突清单里（后端已在源头排除），这里再滤一次，
  // 防止旧版本残留数据把它们带回来误导用户删掉自己的供应商凭据
  const reportable = conflicts.filter((c) => !isManagedEnvKey(c.varName));
  if (reportable.length === 0) {
    return null;
  }

  const selected = reportable.filter((c) =>
    selectedConflicts.has(`${c.varName}:${c.sourcePath}`),
  );

  /** 打开删除确认前先查「这些变量是不是某供应商正在用的凭据」 */
  const openConfirm = async () => {
    setUsage(null);
    setShowConfirmDialog(true);
    try {
      const names = selected.map((c) => c.varName);
      const result = await envVarsInUse(names);
      setUsage(new Map(Object.entries(result)));
    } catch (error) {
      console.warn("[EnvWarningBanner] 查询变量引用失败:", error);
    }
  };

  const inUse = (name: string): string[] =>
    usage?.get(name.toUpperCase()) ?? [];

  const toggleSelection = (key: string) => {
    const newSelection = new Set(selectedConflicts);
    if (newSelection.has(key)) {
      newSelection.delete(key);
    } else {
      newSelection.add(key);
    }
    setSelectedConflicts(newSelection);
  };

  const toggleSelectAll = () => {
    if (selectedConflicts.size === reportable.length) {
      setSelectedConflicts(new Set());
    } else {
      setSelectedConflicts(
        new Set(reportable.map((c) => `${c.varName}:${c.sourcePath}`)),
      );
    }
  };

  const handleDelete = async () => {
    setShowConfirmDialog(false);
    setIsDeleting(true);

    try {
      // 用过滤后的清单，避免把受管键/已剔除条目带进删除请求
      const conflictsToDelete = reportable.filter((c) =>
        selectedConflicts.has(`${c.varName}:${c.sourcePath}`),
      );

      if (conflictsToDelete.length === 0) {
        toast.warning(t("env.error.noSelection"));
        return;
      }

      const backupInfo = await deleteEnvVars(conflictsToDelete);

      toast.success(t("env.delete.success"), {
        description: t("env.backup.location", {
          path: backupInfo.backupPath,
        }),
        duration: 5000,
        closeButton: true,
      });

      // 清空选择并通知父组件
      setSelectedConflicts(new Set());
      setUsage(null);
      onDeleted();
    } catch (error) {
      console.error("删除环境变量失败:", error);
      toast.error(t("env.delete.error"), {
        description: String(error),
      });
    } finally {
      setIsDeleting(false);
    }
  };

  const getSourceDescription = (conflict: EnvConflict): string => {
    if (conflict.sourceType === "system") {
      if (conflict.sourcePath.includes("HKEY_CURRENT_USER")) {
        return t("env.source.userRegistry");
      } else if (conflict.sourcePath.includes("HKEY_LOCAL_MACHINE")) {
        return t("env.source.systemRegistry");
      } else {
        return t("env.source.systemEnv");
      }
    } else {
      return conflict.sourcePath;
    }
  };

  return (
    <>
      <div className="fixed top-0 left-0 right-0 z-[100] bg-yellow-50 dark:bg-yellow-950 border-b border-yellow-200 dark:border-yellow-900 shadow-lg animate-slide-down">
        <div className="container mx-auto px-4 py-3">
          <div className="flex items-start gap-3">
            <AlertTriangle className="h-5 w-5 text-yellow-600 dark:text-yellow-500 flex-shrink-0 mt-0.5" />

            <div className="flex-1 min-w-0">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <h3 className="text-sm font-semibold text-yellow-900 dark:text-yellow-100">
                    {t("env.warning.title")}
                  </h3>
                  <p className="text-sm text-yellow-800 dark:text-yellow-200 mt-0.5">
                    {t("env.warning.description", { count: reportable.length })}
                  </p>
                </div>

                <div className="flex items-center gap-2 flex-shrink-0">
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setIsExpanded(!isExpanded)}
                    className="text-yellow-900 dark:text-yellow-100 hover:bg-yellow-100 dark:hover:bg-yellow-900/50"
                  >
                    {isExpanded ? (
                      <>
                        {t("env.actions.collapse")}
                        <ChevronUp className="h-4 w-4 ml-1" />
                      </>
                    ) : (
                      <>
                        {t("env.actions.expand")}
                        <ChevronDown className="h-4 w-4 ml-1" />
                      </>
                    )}
                  </Button>

                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={onDismiss}
                    className="text-yellow-900 dark:text-yellow-100 hover:bg-yellow-100 dark:hover:bg-yellow-900/50"
                  >
                    <X className="h-4 w-4" />
                  </Button>
                </div>
              </div>

              {isExpanded && (
                <div className="mt-4 space-y-3">
                  <div className="flex items-center gap-2 pb-2 border-b border-yellow-200 dark:border-yellow-900/50">
                    <Checkbox
                      id="select-all"
                      checked={selectedConflicts.size === reportable.length}
                      onCheckedChange={toggleSelectAll}
                    />
                    <label
                      htmlFor="select-all"
                      className="text-sm font-medium text-yellow-900 dark:text-yellow-100 cursor-pointer"
                    >
                      {t("env.actions.selectAll")}
                    </label>
                  </div>

                  <div className="max-h-96 overflow-y-auto space-y-2">
                    {reportable.map((conflict) => {
                      const key = `${conflict.varName}:${conflict.sourcePath}`;
                      return (
                        <div
                          key={key}
                          className="flex items-start gap-3 p-3 bg-white dark:bg-gray-900 rounded-md border border-yellow-200 dark:border-yellow-900/50"
                        >
                          <Checkbox
                            id={key}
                            checked={selectedConflicts.has(key)}
                            onCheckedChange={() => toggleSelection(key)}
                          />

                          <div className="flex-1 min-w-0">
                            <label
                              htmlFor={key}
                              className="block text-sm font-medium text-foreground cursor-pointer"
                            >
                              {conflict.varName}
                            </label>
                            <p className="text-xs text-muted-foreground mt-1 break-all">
                              {t("env.field.value")}: {conflict.varValue}
                            </p>
                            <p className="text-xs text-muted-foreground mt-1">
                              {t("env.field.source")}:{" "}
                              {getSourceDescription(conflict)}
                            </p>
                          </div>
                        </div>
                      );
                    })}
                  </div>

                  <div className="flex items-center justify-end gap-2 pt-2 border-t border-yellow-200 dark:border-yellow-900/50">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => setSelectedConflicts(new Set())}
                      disabled={selectedConflicts.size === 0}
                      className="text-yellow-900 dark:text-yellow-100 border-yellow-300 dark:border-yellow-800"
                    >
                      {t("env.actions.clearSelection")}
                    </Button>

                    <Button
                      variant="destructive"
                      size="sm"
                      onClick={() => void openConfirm()}
                      disabled={selectedConflicts.size === 0 || isDeleting}
                      className="gap-1"
                    >
                      <Trash2 className="h-4 w-4" />
                      {isDeleting
                        ? t("env.actions.deleting")
                        : t("env.actions.deleteSelected", {
                            count: selectedConflicts.size,
                          })}
                    </Button>
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      <Dialog open={showConfirmDialog} onOpenChange={setShowConfirmDialog}>
        <DialogContent className="max-w-md" zIndex="top">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <AlertTriangle className="h-5 w-5 text-destructive" />
              {t("env.confirm.title")}
            </DialogTitle>
            <DialogDescription className="space-y-2">
              <p>
                {t("env.confirm.message", { count: selectedConflicts.size })}
              </p>
              {/* 关键安全提示：这些变量可能正是某个供应商条目里存的凭据 */}
              {selected.length > 0 && (
                <div className="space-y-2 rounded-md border border-destructive/40 bg-destructive/5 p-2">
                  {selected.map((c) => {
                    const users = inUse(c.varName);
                    return (
                      <div key={`${c.varName}:${c.sourcePath}`} className="text-sm">
                        <span className="font-medium">{c.varName}</span>
                        {users.length > 0 ? (
                          <span className="text-destructive">
                            {" "}
                            {t("env.confirm.inUse", {
                              defaultValue: "正在被以下供应商使用",
                            })}
                            ：{users.join("、")}
                          </span>
                        ) : usage ? (
                          <span className="text-muted-foreground">
                            {" "}
                            {t("env.confirm.notInUse", {
                              defaultValue: "未被 OGG 供应商引用",
                            })}
                          </span>
                        ) : null}
                      </div>
                    );
                  })}
                  {usage && selected.some((c) => inUse(c.varName).length > 0) && (
                    <p className="text-sm text-destructive">
                      {t("env.confirm.inUseWarning", {
                        defaultValue:
                          "删除后对应供应商的凭据会失效（卡片还在但用不了），需要重新填写 API Key。",
                      })}
                    </p>
                  )}
                </div>
              )}
              <p className="text-sm text-muted-foreground">
                {t("env.confirm.backupNotice")}
              </p>
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setShowConfirmDialog(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button variant="destructive" onClick={handleDelete}>
              {t("env.confirm.confirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
