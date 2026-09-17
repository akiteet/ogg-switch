/**
 * OMP Role Manager
 * 
 * Manages OMP's 10 semantic model roles.
 * Each role maps to a specific provider/model combination with optional thinking level.
 * 
 * Roles:
 * - default: General-purpose model for most tasks
 * - smol: Fast, cheap model for simple tasks
 * - slow: High-quality, slow model for complex reasoning
 * - plan: Planning and architecture tasks
 * - commit: Git commit message generation
 * - vision: Visual/image understanding
 * - designer: Design-related tasks
 * - task: Background task execution
 * - advisor: Advisory/consulting tasks
 * - tiny: Extremely lightweight model
 */

import { useState, useMemo, useEffect } from "react";
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { 
  Settings, 
  Zap, 
  Brain, 
  Lightbulb, 
  GitCommit, 
  Eye, 
  Paintbrush, 
  Boxes, 
  MessageCircle, 
  Feather,
  Edit,
  Trash2,
  Plus,
} from "lucide-react";
import type {
  OmpRole,
  OmpModelRole,
  OmpProviderConfig,
  OmpModelInfo,
  ThinkingLevel,
} from "@/types/omp";
import { OMP_ROLES } from "@/utils/ompConfig";
import { ompApi } from "@/lib/api";

interface OmpRoleManagerProps {
  providers: OmpProviderConfig[];
  roles: OmpModelRole[];
  onRolesChange: (roles: OmpModelRole[]) => void;
}

// Role metadata
const ROLE_META: Record<
  OmpRole,
  { icon: typeof Settings; label: string; description: string; color: string }
> = {
  default: {
    icon: Settings,
    label: "Default",
    description: "通用模型，适合大多数任务",
    color: "text-blue-600 dark:text-blue-400",
  },
  smol: {
    icon: Zap,
    label: "Smol",
    description: "快速便宜的模型，适合简单任务",
    color: "text-green-600 dark:text-green-400",
  },
  slow: {
    icon: Brain,
    label: "Slow",
    description: "高质量推理模型，适合复杂任务",
    color: "text-purple-600 dark:text-purple-400",
  },
  plan: {
    icon: Lightbulb,
    label: "Plan",
    description: "规划和架构设计专用",
    color: "text-yellow-600 dark:text-yellow-400",
  },
  commit: {
    icon: GitCommit,
    label: "Commit",
    description: "Git 提交消息生成",
    color: "text-orange-600 dark:text-orange-400",
  },
  vision: {
    icon: Eye,
    label: "Vision",
    description: "图像理解和视觉任务",
    color: "text-indigo-600 dark:text-indigo-400",
  },
  designer: {
    icon: Paintbrush,
    label: "Designer",
    description: "设计相关任务",
    color: "text-pink-600 dark:text-pink-400",
  },
  task: {
    icon: Boxes,
    label: "Task",
    description: "后台任务执行",
    color: "text-cyan-600 dark:text-cyan-400",
  },
  advisor: {
    icon: MessageCircle,
    label: "Advisor",
    description: "咨询和建议",
    color: "text-teal-600 dark:text-teal-400",
  },
  tiny: {
    icon: Feather,
    label: "Tiny",
    description: "极轻量级模型",
    color: "text-gray-600 dark:text-gray-400",
  },
};

const THINKING_LEVELS: { value: ThinkingLevel; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "minimal", label: "Minimal" },
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "Extra High" },
  { value: "max", label: "Maximum" },
  { value: "auto", label: "Auto" },
];

/**
 * 供应商在 omp 里的引用 id：
 * - API Key 供应商 = models.yml 的键，即列表条目 id
 * - OAuth 供应商 = 凭据库 id（`omp models --json` 的 provider 字段 / selector 前缀，
 *   例如 openai-codex），而列表条目 id 是 OGG 本地 meta key，两者可能不同。
 */
function providerRefId(provider: OmpProviderConfig): string {
  return provider.type === "oauth"
    ? provider.oauthProviderId ?? provider.id
    : provider.id;
}

export function OmpRoleManager({
  providers,
  roles,
  onRolesChange,
}: OmpRoleManagerProps) {
  const { t } = useTranslation();
  const [editingRole, setEditingRole] = useState<OmpRole | null>(null);
  const [editProviderId, setEditProviderId] = useState("");
  const [editModelId, setEditModelId] = useState("");
  const [editThinkingLevel, setEditThinkingLevel] = useState<ThinkingLevel | undefined>();
  // OAuth 供应商不落 models.yml（合成条目 models 为空），弹窗内按需懒加载
  const [fetchedModels, setFetchedModels] = useState<Record<string, OmpModelInfo[]>>({});
  const [loadingModels, setLoadingModels] = useState(false);

  // Build role map
  const roleMap = useMemo(() => {
    const map = new Map<OmpRole, OmpModelRole>();
    for (const role of roles) {
      map.set(role.role, role);
    }
    return map;
  }, [roles]);

  // 懒加载结果合并进供应商列表（仅填充 models 为空的条目）
  const mergedProviders = useMemo(
    () =>
      providers.map((p) => {
        const fetched = fetchedModels[p.id];
        return fetched && fetched.length > 0 && p.models.length === 0
          ? { ...p, models: fetched }
          : p;
      }),
    [providers, fetchedModels],
  );

  // Get available models for a provider
  const getProviderModels = (providerId: string) => {
    const provider = mergedProviders.find((p) => p.id === providerId);
    return provider?.models ?? [];
  };

  // 打开弹窗且所选供应商 models 为空时，从 omp 目录懒加载
  //（OAuth 供应商凭据在 omp 凭据库，models.yml 不落模型清单）
  useEffect(() => {
    if (!editingRole || !editProviderId) return;
    const provider = mergedProviders.find((p) => p.id === editProviderId);
    if (!provider || provider.models.length > 0) return;
    if (fetchedModels[editProviderId] !== undefined) return;
    let cancelled = false;
    setLoadingModels(true);
    ompApi
      .ompListModels(providerRefId(provider))
      .then((models) => {
        if (cancelled) return;
        setFetchedModels((prev) => ({ ...prev, [editProviderId]: models }));
        // 拉到模型且尚未选中时自动选第一个
        if (models.length > 0) {
          setEditModelId((current) => current || models[0]!.id);
        }
      })
      .catch((err) => {
        console.warn("[OmpRoleManager] lazy model fetch failed:", err);
        if (!cancelled) {
          setFetchedModels((prev) => ({ ...prev, [editProviderId]: [] }));
        }
      })
      .finally(() => {
        if (!cancelled) setLoadingModels(false);
      });
    return () => {
      cancelled = true;
    };
  }, [editingRole, editProviderId, mergedProviders, fetchedModels]);

  const handleEditRole = (role: OmpRole) => {
    const existing = roleMap.get(role);
    if (existing) {
      // 角色里存的是 omp 的引用 id，需映射回本地列表条目 id 才能正确回显
      const localProvider = mergedProviders.find(
        (p) => providerRefId(p) === existing.providerId,
      );
      setEditProviderId(localProvider?.id ?? existing.providerId);
      setEditModelId(existing.modelId);
      setEditThinkingLevel(existing.thinkingLevel);
    } else {
      setEditProviderId("");
      setEditModelId("");
      setEditThinkingLevel(undefined);
    }
    setEditingRole(role);
  };

  const handleSaveRole = async () => {
    if (!editingRole || !editProviderId || !editModelId) return;

    // 落盘用 omp 的引用 id（OAuth 供应商必须是凭据库 id，否则 omp 认不出该角色）
    const selectedProvider = mergedProviders.find((p) => p.id === editProviderId);
    const newRole: OmpModelRole = {
      role: editingRole,
      providerId: selectedProvider
        ? providerRefId(selectedProvider)
        : editProviderId,
      modelId: editModelId,
      thinkingLevel: editThinkingLevel,
    };

    try {
      // Save to backend via OMP API
      await ompApi.setOmpRole(newRole);

      // Update local state
      const newRoles = roles.filter((r) => r.role !== editingRole);
      newRoles.push(newRole);
      onRolesChange(newRoles);
      setEditingRole(null);

      toast.success(
        t("omp.roleManager.saveSuccess", {
          role: ROLE_META[editingRole].label,
          defaultValue: `角色 ${ROLE_META[editingRole].label} 保存成功`,
        }),
      );
    } catch (error) {
      console.error("[OmpRoleManager] Save role error:", error);
      toast.error(
        t("omp.roleManager.saveError", {
          defaultValue: "保存角色失败",
        }),
      );
    }
  };

  const handleDeleteRole = async (role: OmpRole) => {
    try {
      // Delete from backend via OMP API
      await ompApi.deleteOmpRole(role);

      // Update local state
      const newRoles = roles.filter((r) => r.role !== role);
      onRolesChange(newRoles);

      toast.success(
        t("omp.roleManager.deleteSuccess", {
          role: ROLE_META[role].label,
          defaultValue: `角色 ${ROLE_META[role].label} 已删除`,
        }),
      );
    } catch (error) {
      console.error("[OmpRoleManager] Delete role error:", error);
      toast.error(
        t("omp.roleManager.deleteError", {
          defaultValue: "删除角色失败",
        }),
      );
    }
  };

  const handleProviderChange = (providerId: string) => {
    setEditProviderId(providerId);
    // Reset model selection when provider changes
    const models = getProviderModels(providerId);
    if (models.length > 0) {
      setEditModelId(models[0].id);
    } else {
      setEditModelId("");
    }
  };

  const selectedProviderModels = useMemo(() => {
    if (!editProviderId) return [];
    return getProviderModels(editProviderId);
  }, [editProviderId, mergedProviders]);

  const selectedModel = useMemo(() => {
    return selectedProviderModels.find((m) => m.id === editModelId);
  }, [selectedProviderModels, editModelId]);

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-lg font-semibold">
            {t("omp.roleManager.title", { defaultValue: "角色管理" })}
          </h3>
          <p className="text-sm text-muted-foreground">
            {t("omp.roleManager.description", {
              defaultValue: "为 10 个语义角色分配模型",
            })}
          </p>
        </div>
        <Badge variant="secondary">
          {roles.length} / {OMP_ROLES.length} {t("omp.roleManager.configured", { defaultValue: "已配置" })}
        </Badge>
      </div>

      {/* Roles Table */}
      <div className="rounded-md border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-[200px]">
                {t("omp.roleManager.role", { defaultValue: "角色" })}
              </TableHead>
              <TableHead>
                {t("omp.roleManager.assignment", { defaultValue: "分配" })}
              </TableHead>
              <TableHead className="w-[100px] text-right">
                {t("omp.roleManager.actions", { defaultValue: "操作" })}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {OMP_ROLES.map((role) => {
              const assignment = roleMap.get(role);
              const meta = ROLE_META[role];
              const Icon = meta.icon;

              return (
                <TableRow key={role}>
                  <TableCell>
                    <div className="flex items-center gap-2">
                      <Icon className={`h-4 w-4 ${meta.color}`} />
                      <div>
                        <div className="font-medium">{meta.label}</div>
                        <div className="text-xs text-muted-foreground">
                          {meta.description}
                        </div>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell>
                    {assignment ? (
                      <div className="flex items-center gap-2">
                        <code className="text-xs bg-muted px-2 py-1 rounded">
                          {assignment.providerId}/{assignment.modelId}
                          {assignment.thinkingLevel && `:${assignment.thinkingLevel}`}
                        </code>
                      </div>
                    ) : (
                      <span className="text-sm text-muted-foreground">
                        {t("omp.roleManager.notAssigned", { defaultValue: "未分配" })}
                      </span>
                    )}
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleEditRole(role)}
                      >
                        {assignment ? (
                          <Edit className="h-4 w-4" />
                        ) : (
                          <Plus className="h-4 w-4" />
                        )}
                      </Button>
                      {assignment && (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => handleDeleteRole(role)}
                        >
                          <Trash2 className="h-4 w-4 text-red-500" />
                        </Button>
                      )}
                    </div>
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      </div>

      {/* Edit Dialog */}
      <Dialog open={!!editingRole} onOpenChange={(open) => !open && setEditingRole(null)}>
        {/* zIndex="alert"(z-60)：默认 base(z-40) 低于固定头部 z-50，高弹窗顶部
            （max-h-90vh 居中后 ~32px 处）会被半透明头部盖住（标题"被裁切"） */}
        <DialogContent className="overflow-hidden" zIndex="alert">
          <DialogHeader>
            <DialogTitle>
              {editingRole && (
                <div className="flex items-center gap-2">
                  {(() => {
                    const meta = ROLE_META[editingRole];
                    const Icon = meta.icon;
                    return <Icon className={`h-5 w-5 ${meta.color}`} />;
                  })()}
                  {t("omp.roleManager.editRole", {
                    role: editingRole && ROLE_META[editingRole].label,
                    defaultValue: `配置 ${editingRole && ROLE_META[editingRole].label} 角色`,
                  })}
                </div>
              )}
            </DialogTitle>
            <DialogDescription>
              {editingRole && ROLE_META[editingRole].description}
            </DialogDescription>
          </DialogHeader>

          {/* 中部可滚动：弹窗为固定高度(flex) + max-h-[90vh]，内容超高时
              必须由这一层滚动，否则溢出部分会被居中定位裁掉（顶部被窗口边缘切掉） */}
          <div className="space-y-4 px-6 py-4 flex-1 min-h-0 overflow-y-auto">
            {/* Provider Selection */}
            <div className="space-y-2">
              <Label>{t("omp.roleManager.provider", { defaultValue: "Provider" })}</Label>
              <Select value={editProviderId} onValueChange={handleProviderChange}>
                <SelectTrigger>
                  <SelectValue placeholder={t("omp.roleManager.selectProvider", { defaultValue: "选择 Provider" })} />
                </SelectTrigger>
                <SelectContent>
                  {mergedProviders.map((provider) => (
                    <SelectItem key={provider.id} value={provider.id}>
                      {provider.name} ({provider.models.length} models)
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            {/* Model Selection */}
            {editProviderId && (
              <div className="space-y-2">
                <Label>{t("omp.roleManager.model", { defaultValue: "Model" })}</Label>
                {selectedProviderModels.length > 0 ? (
                  <Select value={editModelId} onValueChange={setEditModelId}>
                    <SelectTrigger>
                      <SelectValue placeholder={t("omp.roleManager.selectModel", { defaultValue: "选择 Model" })} />
                    </SelectTrigger>
                    <SelectContent>
                      {selectedProviderModels.map((model) => (
                        <SelectItem key={model.id} value={model.id}>
                          {model.name}
                          {model.reasoning && (
                            <Badge variant="secondary" className="ml-2 text-xs">
                              Reasoning
                            </Badge>
                          )}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                ) : loadingModels ? (
                  <div className="h-9 rounded-md border border-input bg-muted/30" />
                ) : (
                  /* OAuth 供应商的模型由 omp 从上游动态发现、不在静态目录里
                     （omp_list_models 过滤结果为空），提供手动输入兜底——
                     omp 角色选择器本就支持直接引用上游模型 id */
                  <Input
                    value={editModelId}
                    onChange={(e) => setEditModelId(e.target.value.trim())}
                    placeholder={t("omp.roleManager.manualModelPlaceholder", {
                      defaultValue: "手动输入模型 ID，例如 gpt-5.2",
                    })}
                  />
                )}
                {loadingModels ? (
                  <p className="text-xs text-muted-foreground">
                    {t("omp.roleManager.loadingModels", { defaultValue: "正在从 OMP 获取模型…" })}
                  </p>
                ) : (
                  selectedProviderModels.length === 0 && (
                    <p className="text-xs text-muted-foreground">
                      {t("omp.roleManager.manualModelHint", {
                        defaultValue:
                          "该供应商的模型清单无法从 omp 获取，直接输入模型 ID 即可（角色将保存为 provider/model）",
                      })}
                    </p>
                  )
                )}
              </div>
            )}

            {/* Thinking Level (if model supports reasoning) */}
            {selectedModel?.reasoning && (
              <div className="space-y-2">
                <Label>{t("omp.roleManager.thinkingLevel", { defaultValue: "Thinking Level" })}</Label>
                <Select
                  value={editThinkingLevel ?? "auto"}
                  onValueChange={(value) => setEditThinkingLevel(value as ThinkingLevel)}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {THINKING_LEVELS.map((level) => (
                      <SelectItem key={level.value} value={level.value}>
                        {level.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {t("omp.roleManager.thinkingLevelHelp", {
                    defaultValue: "Extended thinking 模型的思考深度",
                  })}
                </p>
              </div>
            )}

            {/* Preview */}
            {editProviderId && editModelId && (
              <div className="rounded-lg bg-muted p-3 text-sm">
                <div className="font-medium mb-1">
                  {t("omp.roleManager.preview", { defaultValue: "预览" })}
                </div>
                <code className="text-xs">
                  {editProviderId}/{editModelId}
                  {editThinkingLevel && `:${editThinkingLevel}`}
                </code>
              </div>
            )}
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setEditingRole(null)}>
              {t("common.cancel", { defaultValue: "取消" })}
            </Button>
            <Button
              onClick={handleSaveRole}
              disabled={!editProviderId || !editModelId}
            >
              {t("common.save", { defaultValue: "保存" })}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
