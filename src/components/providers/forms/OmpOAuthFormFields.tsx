/**
 * OMP OAuth Provider Form Fields
 *
 * OGG Switch 当"遥控器"：驱动本机 omp CLI 自己的凭据库完成 OAuth。
 * - 状态：omp token <provider> --list（后端 omp_auth_status）
 * - 登录：omp auth-broker login <provider>（后端在独立终端启动，OMP 走原生流程）
 * - 登出：omp auth-broker logout <provider>
 * 登录结果落进 OMP 凭据库，omp CLI 立即可用；不自建第二套凭据。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { FormLabel } from "@/components/ui/form";
import { Badge } from "@/components/ui/badge";
import {
  LogIn,
  LogOut,
  CheckCircle2,
  XCircle,
  Loader2,
  RefreshCw,
  AlertTriangle,
} from "lucide-react";
import type { OmpApiProtocol } from "@/types/omp";
import { ompApi } from "@/lib/api";

interface OmpOAuthFormFieldsProps {
  oauthProviderId: string;
  apiProtocol: OmpApiProtocol;
  onApiProtocolChange: (value: OmpApiProtocol) => void;
  isLoggedIn: boolean;
  onLoginStateChange: (value: boolean) => void;
}

const API_PROTOCOLS: { value: OmpApiProtocol; label: string }[] = [
  { value: "openai-completions", label: "OpenAI Completions" },
  { value: "openai-responses", label: "OpenAI Responses" },
  { value: "anthropic-messages", label: "Anthropic Messages" },
  { value: "google-generative-ai", label: "Google Generative AI" },
];

type Phase = "idle" | "waiting" | "success" | "error";

export function OmpOAuthFormFields({
  oauthProviderId,
  apiProtocol,
  onApiProtocolChange,
  isLoggedIn,
  onLoginStateChange,
}: OmpOAuthFormFieldsProps) {
  const { t } = useTranslation();
  const [phase, setPhase] = useState<Phase>("idle");
  const [message, setMessage] = useState<string>("");
  const [accounts, setAccounts] = useState<ompApi.OmpAuthAccount[]>([]);
  const [cliAvailable, setCliAvailable] = useState<boolean | null>(null);
  const [isChecking, setIsChecking] = useState(false);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const pollUntilRef = useRef<number>(0);

  const stopPolling = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
  }, []);

  const applyStatus = useCallback(
    (status: ompApi.OmpAuthStatus) => {
      setCliAvailable(status.cliAvailable);
      setAccounts(status.accounts);
      setMessage(status.message);
      onLoginStateChange(status.loggedIn);
    },
    [onLoginStateChange],
  );

  const refreshStatus = useCallback(async (): Promise<boolean> => {
    const provider = oauthProviderId.trim();
    if (!provider) {
      setAccounts([]);
      setMessage(t("omp.oauth.needProviderId", { defaultValue: "请先填写 OAuth Provider ID" }));
      return false;
    }
    setIsChecking(true);
    try {
      const status = await ompApi.ompAuthStatus(provider);
      applyStatus(status);
      return status.loggedIn;
    } catch (error) {
      console.error("[OmpOAuth] status failed:", error);
      setMessage(String(error));
      return false;
    } finally {
      setIsChecking(false);
    }
  }, [oauthProviderId, applyStatus, t]);

  // provider 变化时刷新一次真实状态
  useEffect(() => {
    void refreshStatus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [oauthProviderId]);

  useEffect(() => () => stopPolling(), [stopPolling]);

  const handleLogin = async () => {
    const provider = oauthProviderId.trim();
    if (!provider) {
      toast.error(t("omp.oauth.needProviderId", { defaultValue: "请先填写 OAuth Provider ID" }));
      return;
    }
    try {
      await ompApi.ompAuthLogin(provider);
      setPhase("waiting");
      setMessage(
        t("omp.oauth.waitingInTerminal", {
          defaultValue: "已打开终端，请在 OMP 窗口中完成授权…",
        }),
      );
      toast.info(
        t("omp.oauth.terminalOpened", {
          defaultValue: "已打开终端，请在 OMP 窗口中完成登录",
        }),
      );
      // 轮询回填：最多 5 分钟，每 3 秒一次
      pollUntilRef.current = Date.now() + 5 * 60 * 1000;
      stopPolling();
      pollRef.current = setInterval(async () => {
        const ok = await refreshStatus();
        if (ok) {
          stopPolling();
          setPhase("success");
          toast.success(t("omp.oauth.loginSuccess", { defaultValue: "登录成功" }));
        } else if (Date.now() > pollUntilRef.current) {
          stopPolling();
          setPhase("error");
          setMessage(
            t("omp.oauth.loginTimeout", {
              defaultValue: "等待超时：未检测到登录完成，请重试",
            }),
          );
        }
      }, 3000);
    } catch (error) {
      console.error("[OmpOAuth] login failed:", error);
      setPhase("error");
      setMessage(String(error));
      toast.error(
        t("omp.oauth.loginFailed", {
          defaultValue: "无法启动 OMP 登录流程",
        }),
      );
    }
  };

  const handleLogout = async () => {
    const provider = oauthProviderId.trim();
    if (!provider) return;
    try {
      await ompApi.ompAuthLogout(provider);
      stopPolling();
      setPhase("idle");
      await refreshStatus();
      toast.success(t("omp.oauth.logoutDone", { defaultValue: "已登出" }));
    } catch (error) {
      console.error("[OmpOAuth] logout failed:", error);
      toast.error(String(error));
    }
  };

  const busy = phase === "waiting" || isChecking;

  return (
    <div className="space-y-4">
      {/* CLI 可用性 */}
      {cliAvailable === false && (
        <div className="flex items-start gap-2 rounded-lg border border-yellow-500/30 bg-yellow-500/10 p-3 text-sm">
          <AlertTriangle className="mt-0.5 h-4 w-4 text-yellow-600" />
          <div className="text-xs text-yellow-700 dark:text-yellow-400">
            <p>
              {t("omp.oauth.cliMissing", {
                defaultValue:
                  "未检测到 Oh My Pi CLI。请先安装 omp（irm https://omp.sh/install.ps1 | iex），否则无法登录。",
              })}
            </p>
            {/* 后端探测失败的具体原因（探测了哪个路径等），便于定位「装了却检测不到」 */}
            {message && <p className="mt-1 break-all opacity-80">{message}</p>}
          </div>
        </div>
      )}

      {/* 登录状态 */}
      <div className="rounded-lg border p-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            {isLoggedIn ? (
              <>
                <CheckCircle2 className="h-5 w-5 text-green-500" />
                <span className="text-sm font-medium">
                  {t("omp.oauth.loggedIn", { defaultValue: "已登录" })}
                </span>
                <Badge variant="secondary" className="bg-green-500/10 text-green-700">
                  {accounts.length > 0
                    ? `${accounts.length} ${t("omp.oauth.accounts", { defaultValue: "个账号" })}`
                    : "Active"}
                </Badge>
              </>
            ) : phase === "waiting" ? (
              <>
                <Loader2 className="h-5 w-5 animate-spin text-primary" />
                <span className="text-sm font-medium">
                  {t("omp.oauth.waiting", { defaultValue: "等待授权中…" })}
                </span>
              </>
            ) : (
              <>
                <XCircle className="h-5 w-5 text-muted-foreground" />
                <span className="text-sm font-medium text-muted-foreground">
                  {t("omp.oauth.notLoggedIn", { defaultValue: "未登录" })}
                </span>
              </>
            )}
          </div>
          <div className="flex gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void refreshStatus()}
              disabled={busy}
            >
              <RefreshCw className={`mr-1 h-3.5 w-3.5 ${isChecking ? "animate-spin" : ""}`} />
              {t("common.refresh", { defaultValue: "刷新" })}
            </Button>
            {isLoggedIn ? (
              <Button type="button" variant="outline" size="sm" onClick={handleLogout}>
                <LogOut className="mr-2 h-4 w-4" />
                {t("omp.oauth.logout", { defaultValue: "登出" })}
              </Button>
            ) : (
              <Button
                type="button"
                variant="default"
                size="sm"
                onClick={handleLogin}
                disabled={busy || cliAvailable === false}
              >
                {phase === "waiting" ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : (
                  <LogIn className="mr-2 h-4 w-4" />
                )}
                {t("omp.oauth.login", { defaultValue: "登录" })}
              </Button>
            )}
          </div>
        </div>

        {message && (
          <p className="mt-3 text-xs text-muted-foreground">{message}</p>
        )}

        {accounts.length > 0 && (
          <ul className="mt-3 space-y-1">
            {accounts.map((acc) => (
              <li
                key={`${acc.index}-${acc.identity}`}
                className="flex items-center gap-2 rounded-md bg-muted/50 px-2 py-1 text-xs"
              >
                <Badge variant="outline" className="shrink-0">
                  #{acc.index}
                </Badge>
                <span className="truncate">{acc.identity}</span>
              </li>
            ))}
          </ul>
        )}
      </div>

      {/* OAuth Provider：固定参数（由预设/既有配置决定，与 omp 凭据库目录对齐），
          不提供下拉或手填——可登录的 provider 只存在于 omp 自己的目录里。 */}
      <div className="space-y-2">
        <FormLabel>
          {t("omp.oauth.providerId", { defaultValue: "OAuth Provider ID" })}
        </FormLabel>
        <div className="flex items-center gap-2">
          <Badge variant="outline" className="font-mono text-sm">
            {oauthProviderId || "—"}
          </Badge>
        </div>
        <p className="text-xs text-muted-foreground">
          {t("omp.oauth.providerIdFixed", {
            defaultValue:
              "此 ID 由 omp 凭据库目录固定（如 anthropic / openai-codex / xai-oauth），已按所选预设自动确定，无需选择",
          })}
        </p>
      </div>

      {/* API 协议 */}
      <div className="space-y-2">
        <FormLabel>{t("omp.apiProtocol", { defaultValue: "API 协议" })}</FormLabel>
        <Select value={apiProtocol} onValueChange={onApiProtocolChange}>
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {API_PROTOCOLS.map((protocol) => (
              <SelectItem key={protocol.value} value={protocol.value}>
                {protocol.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* 模型说明：OAuth 供应商凭据在 omp 凭据库，模型由 omp 从上游自动发现，
          无需（也不应）在 models.yml 配置模型清单。 */}
      <div className="rounded-lg border border-dashed p-4 text-xs text-muted-foreground">
        {t("omp.oauth.modelsAutoDiscover", {
          defaultValue:
            "无需配置模型列表：登录后 OMP 会自动发现该供应商的模型，直接在「角色设置」中选用即可。",
        })}
      </div>

      {/* 说明 */}
      <div className="rounded-lg bg-muted/50 p-4 text-sm">
        <p className="mb-2 font-medium">
          {t("omp.oauth.howItWorks", { defaultValue: "工作原理" })}
        </p>
        <ul className="list-inside list-disc space-y-1 text-xs text-muted-foreground">
          <li>{t("omp.oauth.step1", { defaultValue: "点击「登录」→ 打开终端运行 omp 原生 OAuth 流程" })}</li>
          <li>{t("omp.oauth.step2", { defaultValue: "在终端/浏览器完成授权（浏览器、设备码或粘贴回调）" })}</li>
          <li>{t("omp.oauth.step3", { defaultValue: "凭据存入 OMP 自己的凭据库，omp CLI 立即可用" })}</li>
          <li>{t("omp.oauth.step4", { defaultValue: "本应用自动轮询并回填登录状态与账号列表" })}</li>
        </ul>
      </div>
    </div>
  );
}
