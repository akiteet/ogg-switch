import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Download, Plus, Trash2, User } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { extractErrorMessage } from "@/utils/errorUtils";
import { providersApi, type AntigravityAccount } from "@/lib/api/providers";

/**
 * 认证中心的 Antigravity 账号区块。
 *
 * Google 账号是独立池，不是供应商。官方登录始终对应
 * `antigravity-official`；这里只做导入 / 切换 / 删除快照。
 */
export function AntigravityAuthSection() {
  const { t } = useTranslation();
  const [accounts, setAccounts] = useState<AntigravityAccount[]>([]);
  const [importing, setImporting] = useState(false);

  const reload = useCallback(async () => {
    try {
      setAccounts(await providersApi.listAntigravityAccounts());
    } catch (error) {
      console.warn("[AntigravityAuth] failed to load accounts:", error);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const applyImportResult = useCallback(
    (
      result: Awaited<
        ReturnType<typeof providersApi.importAntigravityFromLive>
      >,
    ) => {
      if (result.outcome === "imported-account") {
        toast.success(
          t("provider.antigravity.importedAccount", {
            defaultValue: "已导入当前 Google 登录账号",
          }),
        );
      } else if (result.outcome === "account-updated") {
        toast.success(
          t("provider.antigravity.accountUpdated", {
            defaultValue: "该账号已存在，登录凭据快照已刷新",
          }),
        );
      } else if (result.outcome === "imported-api-key") {
        toast.success(
          t("provider.antigravity.importedApiKey", {
            defaultValue: "已导入当前 API Key / 中转配置",
          }),
        );
      } else if (result.outcome === "skipped") {
        toast.info(t("provider.noProviders"));
      } else {
        toast.error(
          result.diagnostics ||
            t("provider.antigravity.importFailed", {
              defaultValue: "未检测到可导入的 Google 登录凭据",
            }),
        );
      }
    },
    [t],
  );

  const handleImport = useCallback(async () => {
    setImporting(true);
    try {
      const result = await providersApi.importAntigravityFromLive();
      applyImportResult(result);
      await reload();
    } catch (error) {
      toast.error(extractErrorMessage(error) || t("settings.importFailed"));
    } finally {
      setImporting(false);
    }
  }, [applyImportResult, reload, t]);

  const handleSwitch = useCallback(
    async (account: AntigravityAccount) => {
      try {
        await providersApi.switchAntigravityAccount(account.id);
        await reload();
        toast.success(
          t("notifications.antigravityRestartRequired", {
            defaultValue:
              "切换成功，环境变量已更新；请重开终端 / IDE 并重启 agy 以生效",
          }),
          { closeButton: true },
        );
      } catch (error) {
        toast.error(extractErrorMessage(error) || t("common.unknown"));
      }
    },
    [reload, t],
  );

  const handleDelete = useCallback(
    async (account: AntigravityAccount) => {
      if (
        !window.confirm(
          t("provider.antigravity.deleteConfirm", {
            defaultValue:
              "确定删除该账号快照？（仅删除 OGG Switch 中的快照，不影响 agy 当前登录）",
          }),
        )
      ) {
        return;
      }
      try {
        await providersApi.deleteAntigravityAccount(account.id);
        await reload();
      } catch (error) {
        toast.error(extractErrorMessage(error) || t("common.unknown"));
      }
    },
    [reload, t],
  );

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <Label>
          {t("provider.antigravity.authStatus", {
            defaultValue: "Google 账号",
          })}
        </Label>
        <Badge
          variant={accounts.length > 0 ? "default" : "secondary"}
          className={
            accounts.length > 0 ? "bg-green-500 hover:bg-green-600" : ""
          }
        >
          {accounts.length > 0
            ? t("provider.antigravity.accountCount", {
                count: accounts.length,
                defaultValue: `${accounts.length} 个账号`,
              })
            : t("provider.antigravity.notImported", {
                defaultValue: "未导入",
              })}
        </Badge>
      </div>

      <p className="text-sm text-muted-foreground">
        {t("provider.antigravity.authCenterHint", {
          defaultValue:
            "登录 Google 后即可在此切换账号。切换后请重开终端 / IDE。",
        })}
      </p>

      {accounts.length > 0 && (
        <div className="space-y-1">
          {accounts.map((account) => (
            <div
              key={account.id}
              className="flex items-center justify-between rounded-md border bg-muted/30 p-2"
            >
              <div className="flex min-w-0 items-center gap-2">
                <User className="h-5 w-5 shrink-0 text-muted-foreground" />
                <span className="truncate text-sm font-medium">
                  {account.email || account.name}
                </span>
                {account.isCurrent && (
                  <Badge variant="secondary" className="text-xs">
                    {t("provider.inUse")}
                  </Badge>
                )}
              </div>
              <div className="flex items-center gap-1">
                {!account.isCurrent && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 px-2 text-xs"
                    onClick={() => void handleSwitch(account)}
                  >
                    {t("provider.setAsDefault")}
                  </Button>
                )}
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7 text-muted-foreground hover:text-red-500"
                  onClick={() => void handleDelete(account)}
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            </div>
          ))}
        </div>
      )}

      <Button
        type="button"
        variant="outline"
        className="w-full"
        onClick={() => void handleImport()}
        disabled={importing}
      >
        {accounts.length > 0 ? (
          <Plus className="mr-2 h-4 w-4" />
        ) : (
          <Download className="mr-2 h-4 w-4" />
        )}
        {importing
          ? t("common.loading", { defaultValue: "加载中…" })
          : t("provider.antigravity.importButton", {
              defaultValue: "导入当前账号",
            })}
      </Button>
    </div>
  );
}
