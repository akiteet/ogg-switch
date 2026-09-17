/**
 * OMP API Key Provider Form Fields
 * 
 * API Key providers use traditional API key authentication.
 * Keys can be plain text, environment variables, or secret-bridge commands.
 */

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { FormLabel } from "@/components/ui/form";
import { Badge } from "@/components/ui/badge";
import { Key, DollarSign, FileJson, Eye, EyeOff } from "lucide-react";
import type { OmpApiProtocol, OmpModelInfo } from "@/types/omp";
import { OmpModelListEditor } from "./OmpModelListEditor";

interface OmpApiKeyFormFieldsProps {
  baseUrl: string;
  onBaseUrlChange: (value: string) => void;
  apiKey: string;
  onApiKeyChange: (value: string) => void;
  apiProtocol: OmpApiProtocol;
  onApiProtocolChange: (value: OmpApiProtocol) => void;
  headers?: Record<string, string>;
  onHeadersChange: (value: Record<string, string>) => void;
  authHeader?: boolean;
  onAuthHeaderChange: (value: boolean) => void;
  models: OmpModelInfo[];
  onModelsChange: (value: OmpModelInfo[]) => void;
  providerId?: string;
  /** 预设/供应商的文档或官网链接：有值时在 API Key 下方显示「获取 API Key」 */
  apiKeyLinkUrl?: string;
}

const API_PROTOCOLS: { value: OmpApiProtocol; label: string }[] = [
  { value: "openai-completions", label: "OpenAI Completions" },
  { value: "openai-responses", label: "OpenAI Responses" },
  { value: "anthropic-messages", label: "Anthropic Messages" },
  { value: "google-generative-ai", label: "Google Generative AI" },
];

export function OmpApiKeyFormFields({
  baseUrl,
  onBaseUrlChange,
  apiKey,
  onApiKeyChange,
  apiProtocol,
  onApiProtocolChange,
  headers,
  onHeadersChange,
  authHeader,
  onAuthHeaderChange,
  models,
  onModelsChange,
  providerId,
  apiKeyLinkUrl,
}: OmpApiKeyFormFieldsProps) {
  const { t } = useTranslation();
  const [showKey, setShowKey] = useState(false);

  const headersJson = JSON.stringify(headers ?? {}, null, 2);

  const handleHeadersChange = (value: string) => {
    try {
      const parsed = JSON.parse(value);
      onHeadersChange(parsed);
    } catch {
      // Invalid JSON, ignore
    }
  };

  return (
    <div className="space-y-4">
      {/* Base URL */}
      <div className="space-y-2">
        <FormLabel className="flex items-center gap-2">
          <DollarSign className="h-4 w-4" />
          {t("omp.baseUrl", { defaultValue: "Base URL" })}
        </FormLabel>
        <Input
          type="url"
          value={baseUrl}
          onChange={(e) => onBaseUrlChange(e.target.value)}
          placeholder="https://api.example.com/v1"
          className="font-mono text-sm"
        />
        <p className="text-xs text-muted-foreground">
          {t("omp.baseUrlHelp", {
            defaultValue: "API 服务的基础 URL",
          })}
        </p>
      </div>

      {/* API Key */}
      <div className="space-y-2">
        <FormLabel className="flex items-center gap-2">
          <Key className="h-4 w-4" />
          {t("omp.apiKey", { defaultValue: "API Key" })}
        </FormLabel>
        <div className="relative">
          <Input
            type={showKey ? "text" : "password"}
            value={apiKey}
            onChange={(e) => onApiKeyChange(e.target.value)}
            placeholder="sk-... or $ENV_VAR or $(secret-get ...)"
            className="font-mono text-sm pr-10"
          />
          {apiKey && (
            <button
              type="button"
              onClick={() => setShowKey((v) => !v)}
              className="absolute inset-y-0 right-0 flex items-center pr-3 text-muted-foreground hover:text-foreground transition-colors"
              aria-label={showKey ? t("apiKeyInput.hide", { defaultValue: "隐藏" }) : t("apiKeyInput.show", { defaultValue: "显示" })}
            >
              {showKey ? <EyeOff size={16} /> : <Eye size={16} />}
            </button>
          )}
        </div>
        <div className="flex flex-wrap gap-2">
          <Badge variant="secondary" className="text-xs">
            Plain: sk-abc123...
          </Badge>
          <Badge variant="secondary" className="text-xs">
            Env: $OPENAI_API_KEY
          </Badge>
          <Badge variant="secondary" className="text-xs">
            Secret: $(secret-get openai)
          </Badge>
        </div>
        <p className="text-xs text-muted-foreground">
          {t("omp.apiKeyHelp", {
            defaultValue: "支持明文、环境变量 ($VAR) 或 secret-bridge 命令",
          })}
        </p>
        {/* 快捷跳转官网/文档获取 API Key（对齐 cc-switch 的表单） */}
        {apiKeyLinkUrl && (
          <a
            href={apiKeyLinkUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="text-xs text-blue-400 dark:text-blue-500 hover:text-blue-500 dark:hover:text-blue-400 transition-colors"
          >
            {t("providerForm.getApiKey", { defaultValue: "获取 API Key" })}
          </a>
        )}
      </div>

      {/* API Protocol */}
      <div className="space-y-2">
        <FormLabel>
          {t("omp.apiProtocol", { defaultValue: "API 协议" })}
        </FormLabel>
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

      {/* Auth Header */}
      <div className="flex items-center space-x-2">
        <Checkbox
          id="authHeader"
          checked={authHeader}
          onCheckedChange={(checked) => onAuthHeaderChange(!!checked)}
        />
        <label
          htmlFor="authHeader"
          className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
        >
          {t("omp.sendAuthHeader", {
            defaultValue: "发送 Authorization header",
          })}
        </label>
      </div>

      {/* Custom Headers (Optional) */}
      <div className="space-y-2">
        <FormLabel className="flex items-center gap-2">
          <FileJson className="h-4 w-4" />
          {t("omp.customHeaders", { defaultValue: "自定义 Headers（可选）" })}
        </FormLabel>
        <Textarea
          value={headersJson}
          onChange={(e) => handleHeadersChange(e.target.value)}
          placeholder={`{\n  "X-Custom-Header": "value"\n}`}
          className="font-mono text-xs"
          rows={4}
        />
        <p className="text-xs text-muted-foreground">
          {t("omp.customHeadersHelp", {
            defaultValue: "JSON 格式的额外 HTTP headers",
          })}
        </p>
      </div>

      {/* Models */}
      <OmpModelListEditor
        models={models}
        onModelsChange={onModelsChange}
        baseUrl={baseUrl}
        apiKey={apiKey}
        authHeader={authHeader}
        providerId={providerId}
      />
    </div>
  );
}
