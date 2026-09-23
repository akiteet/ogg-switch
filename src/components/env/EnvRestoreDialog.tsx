import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Loader2, RotateCcw } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  diffEnvBackup,
  listEnvBackups,
  restoreEnvBackup,
} from "@/lib/api/env";
import type { EnvBackupSummary, EnvRestoreDiff } from "@/types/env";

/**
 * 环境变量备份恢复。
 *
 * 之前 `restoreEnvBackup` 只有 API、没有 UI 入口——备份写了却没法恢复（实际踩过）。
 * 这里补上，并且**先对比再写入**：恢复是无条件覆盖，如果机器上后来又切换过供应商，
 * 直接恢复会把新值顶掉；对比表让用户看清每一条「当前是什么 / 会变成什么」。
 */
export function EnvRestoreDialog({
  open,
  onOpenChange,
  onRestored,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRestored?: () => void;
}) {
  const { t } = useTranslation();
  const [backups, setBackups] = useState<EnvBackupSummary[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState<EnvRestoreDiff | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isRestoring, setIsRestoring] = useState(false);

  const reload = useCallback(async () => {
    setIsLoading(true);
    try {
      const list = await listEnvBackups();
      setBackups(list);
      setSelected((prev) => prev ?? list[0]?.backupPath ?? null);
    } catch (error) {
      console.error("[EnvRestore] 读取备份列表失败:", error);
      toast.error(
        t("env.restore.listFailed", { defaultValue: "读取备份列表失败" }),
      );
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  useEffect(() => {
    if (open) void reload();
  }, [open, reload]);

  // 选中备份后取对比（只读，不写任何东西）
  useEffect(() => {
    if (!open || !selected) {
      setDiff(null);
      return;
    }
    let cancelled = false;
    diffEnvBackup(selected)
      .then((result) => {
        if (!cancelled) setDiff(result);
      })
      .catch((error) => {
        console.error("[EnvRestore] 读取备份失败:", error);
        if (!cancelled) setDiff(null);
      });
    return () => {
      cancelled = true;
    };
  }, [open, selected]);

  const handleRestore = async () => {
    if (!selected) return;
    setIsRestoring(true);
    try {
      await restoreEnvBackup(selected);
      toast.success(
        t("env.restore.success", { defaultValue: "环境变量已从备份恢复" }),
      );
      onRestored?.();
      onOpenChange(false);
    } catch (error) {
      console.error("[EnvRestore] 恢复失败:", error);
      toast.error(t("env.restore.error", { defaultValue: "恢复环境变量失败" }), {
        description: String(error),
      });
    } finally {
      setIsRestoring(false);
    }
  };

  const overrides = diff?.entries.filter((e) => e.differs) ?? [];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg" zIndex="top">
        <DialogHeader>
          <DialogTitle>
            {t("env.restore.title", { defaultValue: "恢复环境变量备份" })}
          </DialogTitle>
          <DialogDescription>
            {t("env.restore.description", {
              defaultValue:
                "选择一份备份，先确认下面的对比再恢复——恢复会无条件覆盖当前值。",
            })}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3 max-h-[50vh] overflow-y-auto">
          {isLoading ? (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("env.restore.loading", { defaultValue: "读取备份…" })}
            </div>
          ) : backups.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {t("env.restore.empty", { defaultValue: "暂无环境变量备份" })}
            </p>
          ) : (
            <>
              <div className="flex flex-wrap gap-2">
                {backups.map((backup) => (
                  <Button
                    key={backup.backupPath}
                    size="sm"
                    variant={
                      selected === backup.backupPath ? "default" : "outline"
                    }
                    onClick={() => setSelected(backup.backupPath)}
                  >
                    {backup.timestamp}
                  </Button>
                ))}
              </div>

              {diff && (
                <div className="space-y-2">
                  {overrides.length > 0 && (
                    <p className="text-sm text-destructive">
                      {t("env.restore.overrideWarning", {
                        count: overrides.length,
                        defaultValue:
                          "{{count}} 个变量的当前值与备份不同，恢复后会被覆盖。",
                      })}
                    </p>
                  )}
                  {diff.entries.map((entry) => (
                    <div
                      key={entry.varName}
                      className="rounded-md border p-2 text-xs space-y-1"
                    >
                      <div className="font-medium">
                        {entry.varName}
                        {entry.managed && (
                          <span className="ml-2 text-muted-foreground">
                            {t("env.restore.managed", {
                              defaultValue: "（OGG 受管）",
                            })}
                          </span>
                        )}
                      </div>
                      <div className="text-muted-foreground break-all">
                        {t("env.restore.backupValue", {
                          defaultValue: "备份值",
                        })}
                        ：{entry.backupValue}
                      </div>
                      <div className="text-muted-foreground break-all">
                        {t("env.restore.currentValue", {
                          defaultValue: "当前值",
                        })}
                        ：
                        {entry.currentValue ??
                          t("env.restore.notSet", { defaultValue: "未设置" })}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            onClick={() => void handleRestore()}
            disabled={!selected || isRestoring || !diff}
            className="gap-1"
          >
            {isRestoring ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <RotateCcw className="h-4 w-4" />
            )}
            {t("env.restore.confirm", { defaultValue: "确认恢复" })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
