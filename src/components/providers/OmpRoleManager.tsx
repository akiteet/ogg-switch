/**
 * OMP Role Manager
 *
 * 管理 OMP config.yml 的 modelRoles。角色集合与 OMP 18.x 内置表对齐
 * （`config/model-roles.ts` 的 MODEL_ROLES）：
 *
 * chat 区（默认路由到对话模型）
 * - default：通用模型
 * - smol：快速便宜的模型
 * - slow：高质量推理模型
 * - vision：图像理解
 * - plan：规划/架构
 * - commit：Git 提交消息
 * - tiny：极轻量任务
 * - memory：记忆/历史压缩
 * - task：子任务
 * - advisor：咨询建议
 *
 * kind 区（按模型种类路由，取值常指向 OMP 的合成供应商 web / local）
 * - image：图像生成
 * - web：联网搜索
 * - speech：语音合成（tts）
 * - dictation：语音识别（stt）
 * - judge：评审/判定
 *
 * config.yml 里还可能有自定义角色键（如已废弃的 designer），它们同样出现在
 * 本列表里，不做过滤——否则用户看得见 OMP 里的角色却在 OGG 里改不了。
 */

import { Fragment, useState, useMemo, useEffect } from "react";
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
  Database,
  Boxes,
  MessageCircle,
  Feather,
  Image as ImageIcon,
  Globe,
  Volume2,
  Mic,
  Scale,
  Layers,
  Edit,
  Trash2,
  Plus,
} from "lucide-react";
import type {
  OmpRole,
  OmpModelRole,
  OmpProviderConfig,
  OmpModelInfo,
  OmpEnabledProvider,
  ThinkingLevel,
} from "@/types/omp";
import { OMP_CHAT_ROLES, OMP_KIND_ROLES } from "@/utils/ompConfig";
import { ompApi } from "@/lib/api";

interface OmpRoleManagerProps {
  providers: OmpProviderConfig[];
  roles: OmpModelRole[];
  onRolesChange: (roles: OmpModelRole[]) => void;
}

type RoleMeta = {
  icon: typeof Settings;
  label: string;
  descKey: string;
  color: string;
  /**
   * 该角色接受的模型 kind（OMP `model-roles.ts` 的 accepts 规则）。
   * 空数组 = 不筛选。models.yml 来的模型没有 kind，按 chat 处理。
   */
  accepts: readonly string[];
};

// Role metadata（label 用 OMP 官方 name，便于与 OMP TUI 对照）
// description 存 i18n key（四语言翻译见 locales 的 omp.roleManager.roleDesc.*）
const ROLE_META: Record<OmpRole, RoleMeta> = {
  default: {
    icon: Settings,
    label: "Default",
    descKey: "omp.roleManager.roleDesc.default",
    color: "text-blue-600 dark:text-blue-400",
    accepts: ["chat"],
  },
  smol: {
    icon: Zap,
    label: "Smol (Fast)",
    descKey: "omp.roleManager.roleDesc.smol",
    color: "text-green-600 dark:text-green-400",
    accepts: ["chat"],
  },
  slow: {
    icon: Brain,
    label: "Slow (Thinking)",
    descKey: "omp.roleManager.roleDesc.slow",
    color: "text-purple-600 dark:text-purple-400",
    accepts: ["chat"],
  },
  vision: {
    icon: Eye,
    label: "Vision",
    descKey: "omp.roleManager.roleDesc.vision",
    color: "text-indigo-600 dark:text-indigo-400",
    accepts: ["chat"],
  },
  plan: {
    icon: Lightbulb,
    label: "Plan (Architect)",
    descKey: "omp.roleManager.roleDesc.plan",
    color: "text-yellow-600 dark:text-yellow-400",
    accepts: ["chat"],
  },
  commit: {
    icon: GitCommit,
    label: "Commit",
    descKey: "omp.roleManager.roleDesc.commit",
    color: "text-orange-600 dark:text-orange-400",
    accepts: ["chat"],
  },
  tiny: {
    icon: Feather,
    label: "Tiny",
    descKey: "omp.roleManager.roleDesc.tiny",
    color: "text-gray-600 dark:text-gray-400",
    accepts: ["chat", "tiny"],
  },
  memory: {
    icon: Database,
    label: "Memory",
    descKey: "omp.roleManager.roleDesc.memory",
    color: "text-rose-600 dark:text-rose-400",
    accepts: ["chat", "tiny"],
  },
  task: {
    icon: Boxes,
    label: "Task (Subtask)",
    descKey: "omp.roleManager.roleDesc.task",
    color: "text-cyan-600 dark:text-cyan-400",
    accepts: ["chat"],
  },
  advisor: {
    icon: MessageCircle,
    label: "Advisor",
    descKey: "omp.roleManager.roleDesc.advisor",
    color: "text-teal-600 dark:text-teal-400",
    accepts: ["chat"],
  },
  image: {
    icon: ImageIcon,
    label: "Image",
    descKey: "omp.roleManager.roleDesc.image",
    color: "text-pink-600 dark:text-pink-400",
    accepts: ["image"],
  },
  web: {
    icon: Globe,
    label: "Web",
    descKey: "omp.roleManager.roleDesc.web",
    color: "text-sky-600 dark:text-sky-400",
    // OMP 的 acceptsWeb 是「search 或带 webSearch 标记的 chat 模型」，而 webSearch
    // 标记在目录 JSON 里看不到；只列 search 才能保证选出来的值一定被 OMP 采纳。
    accepts: ["search"],
  },
  speech: {
    icon: Volume2,
    label: "Speech",
    descKey: "omp.roleManager.roleDesc.speech",
    color: "text-amber-600 dark:text-amber-400",
    accepts: ["tts"],
  },
  dictation: {
    icon: Mic,
    label: "Dictation",
    descKey: "omp.roleManager.roleDesc.dictation",
    color: "text-violet-600 dark:text-violet-400",
    accepts: ["stt"],
  },
  judge: {
    icon: Scale,
    label: "Judge",
    descKey: "omp.roleManager.roleDesc.judge",
    color: "text-lime-600 dark:text-lime-400",
    accepts: ["judge", "tiny", "chat"],
  },
};

/** 取角色元数据：内置角色用 ROLE_META，自定义角色用兜底条目。 */
function roleMeta(role: string): RoleMeta {
  const builtIn = (ROLE_META as Record<string, RoleMeta>)[role];
  if (builtIn) return builtIn;
  return {
    icon: Layers,
    label: role,
    // 复用自定义角色分组的 hint 文案（语义相同，不另开 key）
    descKey: "omp.roleManager.groupCustomHint",
    color: "text-muted-foreground",
    accepts: [],
  };
}

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

/** models.yml 来的模型没有 kind，按 chat 处理（OGG 写入的条目都是对话模型） */
function modelKind(model: OmpModelInfo): string {
  return model.kind ?? "chat";
}

export function OmpRoleManager({
  providers,
  roles,
  onRolesChange,
}: OmpRoleManagerProps) {
  const { t } = useTranslation();
  const [editingRole, setEditingRole] = useState<string | null>(null);
  /** 新建自定义角色：true 时弹窗多一个「角色名称」输入，保存时用它当角色键 */
  const [creatingRole, setCreatingRole] = useState(false);
  const [newRoleName, setNewRoleName] = useState("");
  const [editProviderId, setEditProviderId] = useState("");
  const [editModelId, setEditModelId] = useState("");
  const [editThinkingLevel, setEditThinkingLevel] = useState<ThinkingLevel | undefined>();
  // OMP 目录里已启用的供应商（web / local 等 models.yml 之外的条目）；
  // 角色选择器必须能选到它们，否则 web/parallel、local/kokoro 这类取值无法配置。
  const [enabledProviders, setEnabledProviders] = useState<OmpEnabledProvider[]>([]);
  // 弹窗内按需从 OMP 目录取候选模型（key = `<providerRef>`），取不到时回落 models.yml 里的清单
  const [catalogModels, setCatalogModels] = useState<Record<string, OmpModelInfo[]>>({});

  useEffect(() => {
    let cancelled = false;
    ompApi
      .ompListEnabledProviders()
      .then((list) => {
        if (!cancelled) setEnabledProviders(list);
      })
      .catch((error) => {
        console.warn("[OmpRoleManager] list enabled providers failed:", error);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // 角色行 = 内置 15 个 + 配置里出现的自定义角色（排序稳定，自定义在后）
  const knownRoles = useMemo(() => {
    const seen = new Set<string>(OMP_CHAT_ROLES);
    for (const role of OMP_KIND_ROLES) seen.add(role);
    const custom = roles
      .map((r) => r.role)
      .filter((role) => role && !seen.has(role))
      .filter((role, index, all) => all.indexOf(role) === index)
      .sort((a, b) => a.localeCompare(b));
    return [
      ...OMP_CHAT_ROLES.map((role) => role as string),
      ...OMP_KIND_ROLES.map((role) => role as string),
      ...custom,
    ];
  }, [roles]);

  const roleMap = useMemo(() => {
    const map = new Map<string, OmpModelRole>();
    for (const role of roles) {
      map.set(role.role, role);
    }
    return map;
  }, [roles]);

  // 供应商下拉项：OGG 的供应商 + OMP 目录里 OGG 不认识的（web / local / 未导入的 OAuth）
  const providerOptions = useMemo(() => {
    const rows = providers.map((provider) => ({
      /** 下拉 value / 本地条目 id */
      id: provider.id,
      /** 写进 selector 的引用 id */
      ref: providerRefId(provider),
      label: `${provider.name} (${provider.models.length} models)`,
    }));
    const known = new Set(rows.map((row) => row.ref));
    for (const enabled of enabledProviders) {
      if (known.has(enabled.id)) continue;
      known.add(enabled.id);
      rows.push({
        id: enabled.id,
        ref: enabled.id,
        label: `${enabled.id} · ${t("omp.roleManager.ompBuiltinProvider", {
          defaultValue: "OMP 内置",
        })} (${enabled.modelCount} models)`,
      });
    }
    // 目录查询不可用（omp CLI 缺失 / models.yml 校验失败）时，已存在的角色分配
    // 仍要能显示出来，否则下拉会空着而值其实还在
    if (editProviderId && !rows.some((row) => row.id === editProviderId)) {
      rows.push({
        id: editProviderId,
        ref: editProviderId,
        label: `${editProviderId} · ${t("omp.roleManager.ompBuiltinProvider", {
          defaultValue: "OMP 内置",
        })}`,
      });
    }
    return rows;
  }, [providers, enabledProviders, editProviderId, t]);

  const refForProviderId = (providerId: string): string =>
    providerOptions.find((option) => option.id === providerId)?.ref ?? providerId;

  /** 供应商在 models.yml 里已配置的模型（目录取不到时的回落） */
  const configuredModels = (providerId: string): OmpModelInfo[] =>
    providers.find((p) => p.id === providerId)?.models ?? [];

  // 打开弹窗且所选供应商的目录候选尚未缓存时，从 OMP 目录懒加载（kind=all）。
  // 目录只是**补充**：该供应商在 models.yml 里已配置的模型始终参与候选——
  // models.yml 的条目没有 kind 字段（OMP 把它们一律当 chat），按 kind 过滤会把
  // 用户给 Image / Speech 等角色配置的模型全部滤掉。
  useEffect(() => {
    if (!editingRole || !editProviderId) return;
    const ref = refForProviderId(editProviderId);
    if (!ref) return;
    if (catalogModels[ref] !== undefined) return;
    let cancelled = false;
    ompApi
      .ompListModels(ref, "all")
      .then((models) => {
        if (!cancelled) setCatalogModels((prev) => ({ ...prev, [ref]: models }));
      })
      .catch((err) => {
        console.warn("[OmpRoleManager] lazy model fetch failed:", err);
        if (!cancelled) setCatalogModels((prev) => ({ ...prev, [ref]: [] }));
      });
    return () => {
      cancelled = true;
    };
  }, [editingRole, editProviderId, catalogModels, providers, enabledProviders]);

  const handleEditRole = (role: string) => {
    const existing = roleMap.get(role);
    if (existing) {
      // 角色里存的是 omp 的引用 id，需映射回下拉项 id 才能正确回显
      const option = providerOptions.find((o) => o.ref === existing.providerId);
      setEditProviderId(option?.id ?? existing.providerId);
      setEditModelId(existing.modelId);
      setEditThinkingLevel(existing.thinkingLevel);
    } else {
      setEditProviderId("");
      setEditModelId("");
      setEditThinkingLevel(undefined);
    }
    setCreatingRole(false);
    setNewRoleName("");
    setEditingRole(role);
  };

  const handleCreateRole = () => {
    setEditProviderId("");
    setEditModelId("");
    setEditThinkingLevel(undefined);
    setNewRoleName("");
    setCreatingRole(true);
    setEditingRole("__new__");
  };

  /** 新建自定义角色时以输入框里的名称为准（合法性与重复校验在此完成） */
  const effectiveRole = creatingRole ? newRoleName.trim() : editingRole;

  const acceptedKinds = useMemo<readonly string[]>(
    () => (effectiveRole ? roleMeta(effectiveRole).accepts : []),
    [effectiveRole],
  );

  const roleNameError = useMemo(() => {
    if (!creatingRole) return null;
    const name = newRoleName.trim();
    if (!name) {
      return t("omp.roleManager.roleNameRequired", {
        defaultValue: "请输入角色名称",
      });
    }
    if (/\s/.test(name)) {
      return t("omp.roleManager.roleNameNoWhitespace", {
        defaultValue: "角色名称不能包含空格",
      });
    }
    if (roles.some((r) => r.role === name)) {
      return t("omp.roleManager.roleNameDuplicate", {
        defaultValue: "该角色已存在",
      });
    }
    return null;
  }, [creatingRole, newRoleName, roles, t]);

  const handleSaveRole = async () => {
    if (roleNameError) return;
    if (!effectiveRole || !editProviderId || !editModelId) return;

    // 落盘用 omp 的引用 id（OAuth 供应商必须是凭据库 id，否则 omp 认不出该角色）
    const newRole: OmpModelRole = {
      role: effectiveRole,
      providerId: refForProviderId(editProviderId),
      modelId: editModelId,
      thinkingLevel: editThinkingLevel,
    };

    try {
      // Save to backend via OMP API
      await ompApi.setOmpRole(newRole);

      // Update local state
      const newRoles = roles.filter((r) => r.role !== effectiveRole);
      newRoles.push(newRole);
      onRolesChange(newRoles);
      setEditingRole(null);

      toast.success(
        t("omp.roleManager.saveSuccess", {
          role: roleMeta(effectiveRole).label,
          defaultValue: `角色 ${roleMeta(effectiveRole).label} 保存成功`,
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

  const handleDeleteRole = async (role: string) => {
    try {
      // Delete from backend via OMP API
      await ompApi.deleteOmpRole(role);

      // Update local state
      const newRoles = roles.filter((r) => r.role !== role);
      onRolesChange(newRoles);

      toast.success(
        t("omp.roleManager.deleteSuccess", {
          role: roleMeta(role).label,
          defaultValue: `角色 ${roleMeta(role).label} 已删除`,
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
    // Reset model selection when provider changes（候选由懒加载 effect 填充）
    setEditModelId("");
  };

  // 候选模型 = 该供应商已配置的模型 ∪ OMP 目录条目，**统一**按角色接受的 kind 过滤。
  //
  // 为什么已配置的模型也要过滤：OMP 只在「该角色的候选池」里解析角色取值
  // （roleCandidatePool = 可用模型 ∩ accepts），而 models.yml 条目没有 kind 字段、
  // 一律被当成 chat。把中转站的 chat 模型挂到 image / speech / dictation / web 上，
  // 值会写进 config.yml 但 OMP 不会采纳（回落到自动选择）——列出来只会误导。
  // chat 系角色不受影响：models.yml 条目 kind 缺省即 chat，本来就在 accepts 内。
  const configuredForProvider = editProviderId
    ? configuredModels(editProviderId)
    : [];
  const selectedProviderModels = useMemo(() => {
    if (!editProviderId) return [];
    const configured = configuredModels(editProviderId);
    const configuredIds = new Set(configured.map((m) => m.id));
    const catalog = catalogModels[refForProviderId(editProviderId)] ?? [];
    const merged = [
      ...configured,
      ...catalog.filter((m) => !configuredIds.has(m.id)),
    ];
    if (acceptedKinds.length === 0) return merged;
    return merged.filter((m) => acceptedKinds.includes(modelKind(m)));
  }, [editProviderId, catalogModels, acceptedKinds, providers]);

  // 供应商自己配了模型、但因为 kind 不匹配被全部滤掉：给一句原因，而不是让用户
  // 以为「我明明配了模型，界面却不认」
  const hiddenByKind = useMemo(() => {
    if (acceptedKinds.length === 0 || configuredForProvider.length === 0) {
      return 0;
    }
    return configuredForProvider.filter(
      (m) => !acceptedKinds.includes(modelKind(m)),
    ).length;
  }, [configuredForProvider, acceptedKinds]);

  // 目录是否还查过：直接由缓存判定，不用独立的 loading 布尔——请求落地会触发
  // 重渲染 → effect 清理把 cancelled 置真，若用 setLoading(false) 有概率被吞掉，
  // 空候选时会永远停在骨架屏
  const loadingModels =
    Boolean(editProviderId) &&
    catalogModels[refForProviderId(editProviderId)] === undefined;

  const selectedModel = useMemo(() => {
    return selectedProviderModels.find((m) => m.id === editModelId);
  }, [selectedProviderModels, editModelId]);

  // 表格分组：chat / kind / 自定义（自定义组的行也可能落在 chat 或 kind 的语义里，
  // 但角色名不是内置的，单独成组更清楚）
  const roleGroups = useMemo(() => {
    const builtIn = new Set<string>([...OMP_CHAT_ROLES, ...OMP_KIND_ROLES]);
    const custom = knownRoles.filter((role) => !builtIn.has(role));
    return [
      {
        key: "chat",
        label: t("omp.roleManager.groupChat", { defaultValue: "对话角色" }),
        hint: t("omp.roleManager.groupChatHint", {
          defaultValue: "默认路由到对话模型",
        }),
        roles: OMP_CHAT_ROLES.map((role) => role as string),
      },
      {
        key: "kind",
        label: t("omp.roleManager.groupKind", {
          defaultValue: "种类角色（kind）",
        }),
        hint: t("omp.roleManager.groupKindHint", {
          defaultValue: "图像生成 / 联网搜索 / 语音合成 / 语音识别 / 评审",
        }),
        roles: OMP_KIND_ROLES.map((role) => role as string),
      },
      ...(custom.length > 0
        ? [
            {
              key: "custom",
              label: t("omp.roleManager.groupCustom", {
                defaultValue: "自定义角色",
              }),
              hint: t("omp.roleManager.groupCustomHint", {
                defaultValue: "config.yml 里的自定义 modelRoles 键",
              }),
              roles: custom,
            },
          ]
        : []),
    ];
  }, [knownRoles, t]);

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
              count: knownRoles.length,
              defaultValue: "为 {{count}} 个角色分配模型（内置角色 + 自定义角色）",
            })}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Badge variant="secondary">
            {roles.length} / {knownRoles.length}{" "}
            {t("omp.roleManager.configured", { defaultValue: "已配置" })}
          </Badge>
          <Button type="button" size="sm" variant="outline" onClick={handleCreateRole}>
            <Plus className="mr-1 h-3.5 w-3.5" />
            {t("omp.roleManager.addCustomRole", { defaultValue: "添加自定义角色" })}
          </Button>
        </div>
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
            {roleGroups.map((group) => (
              <Fragment key={`group-${group.key}`}>
                <TableRow className="hover:bg-transparent">
                  <TableCell
                    colSpan={3}
                    className="bg-muted/40 py-1.5 text-xs font-medium text-muted-foreground"
                  >
                    {group.label}
                    <span className="ml-1 font-normal">· {group.hint}</span>
                  </TableCell>
                </TableRow>
                {group.roles.map((role) => {
                  const assignment = roleMap.get(role);
                  const meta = roleMeta(role);
                  const Icon = meta.icon;

                  return (
                    <TableRow key={role}>
                      <TableCell>
                        <div className="flex items-center gap-2">
                          <Icon className={`h-4 w-4 ${meta.color}`} />
                          <div>
                            <div className="font-medium">{meta.label}</div>
                            <div className="text-xs text-muted-foreground">
                              {t(meta.descKey)}
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
              </Fragment>
            ))}
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
                    const meta = roleMeta(effectiveRole ?? editingRole);
                    const Icon = meta.icon;
                    return <Icon className={`h-5 w-5 ${meta.color}`} />;
                  })()}
                  {t("omp.roleManager.editRole", {
                    role: roleMeta(effectiveRole ?? editingRole).label,
                    defaultValue: `配置 ${roleMeta(effectiveRole ?? editingRole).label} 角色`,
                  })}
                </div>
              )}
            </DialogTitle>
            <DialogDescription>
              {editingRole && t(roleMeta(effectiveRole ?? editingRole).descKey)}
            </DialogDescription>
          </DialogHeader>

          {/* 中部可滚动：弹窗为固定高度(flex) + max-h-[90vh]，内容超高时
              必须由这一层滚动，否则溢出部分会被居中定位裁掉（顶部被窗口边缘切掉） */}
          <div className="space-y-4 px-6 py-4 flex-1 min-h-0 overflow-y-auto">
            {/* Role Name（仅新建自定义角色时显示；OMP 对角色键只要求非空，任意名字合法） */}
            {creatingRole && (
              <div className="space-y-2">
                <Label>{t("omp.roleManager.roleName", { defaultValue: "角色名称" })}</Label>
                <Input
                  value={newRoleName}
                  onChange={(e) => setNewRoleName(e.target.value)}
                  placeholder={t("omp.roleManager.roleNamePlaceholder", {
                    defaultValue: "例如 reviewer、writer",
                  })}
                  autoFocus
                />
                {roleNameError ? (
                  <p className="text-xs text-destructive">{roleNameError}</p>
                ) : (
                  <p className="text-xs text-muted-foreground">
                    {t("omp.roleManager.roleNameHint", {
                      defaultValue:
                        "保存后写入 config.yml 的 modelRoles，可与内置角色一样分配模型",
                    })}
                  </p>
                )}
              </div>
            )}

            {/* Provider Selection */}
            <div className="space-y-2">
              <Label>{t("omp.roleManager.provider", { defaultValue: "Provider" })}</Label>
              <Select value={editProviderId} onValueChange={handleProviderChange}>
                <SelectTrigger>
                  <SelectValue placeholder={t("omp.roleManager.selectProvider", { defaultValue: "选择 Provider" })} />
                </SelectTrigger>
                <SelectContent>
                  {providerOptions.map((option) => (
                    <SelectItem key={option.id} value={option.id}>
                      {option.label}
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
                          {model.kind && model.kind !== "chat" && (
                            <Badge variant="secondary" className="ml-2 text-xs">
                              {model.kind}
                            </Badge>
                          )}
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
                  /* models.yml 与目录都拿不到候选（OAuth 供应商的模型由 omp 从上游
                     动态发现）时提供手动输入兜底——omp 角色选择器本就支持直接引用
                     上游模型 id */
                  <Input
                    value={editModelId}
                    onChange={(e) => setEditModelId(e.target.value.trim())}
                    placeholder={t("omp.roleManager.manualModelPlaceholder", {
                      defaultValue: "手动输入模型 ID，例如 gpt-5.2",
                    })}
                  />
                )}
                {/* 目录还在取时只提示进度，不阻塞已给出的候选 */}
                {loadingModels && selectedProviderModels.length > 0 && (
                  <p className="text-xs text-muted-foreground">
                    {t("omp.roleManager.loadingModels", { defaultValue: "正在从 OMP 获取模型…" })}
                  </p>
                )}
                {!loadingModels && hiddenByKind > 0 && (
                  <p className="text-xs text-muted-foreground">
                    {t("omp.roleManager.kindFilteredHint", {
                      kinds: acceptedKinds.join(" / "),
                      count: hiddenByKind,
                      defaultValue:
                        "该角色只接受 OMP 目录里 kind = {{kinds}} 的模型，已隐藏该供应商的 {{count}} 个自建模型（models.yml 条目在 OMP 眼里都是 chat，分配过去不会被采纳）",
                    })}
                  </p>
                )}
                {!loadingModels && selectedProviderModels.length === 0 && (
                  <p className="text-xs text-muted-foreground">
                    {t("omp.roleManager.manualModelHint", {
                      defaultValue:
                        "该供应商没有该角色可用的模型，直接输入模型 ID 即可（角色将保存为 provider/model）",
                    })}
                  </p>
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
                  {refForProviderId(editProviderId)}/{editModelId}
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
              disabled={!effectiveRole || !editProviderId || !editModelId || !!roleNameError}
            >
              {t("common.save", { defaultValue: "保存" })}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
