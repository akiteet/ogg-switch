import { invoke } from "@tauri-apps/api/core";
import type {
  EnvConflict,
  BackupInfo,
  EnvRestoreDiff,
} from "@/types/env";

/**
 * 环境变量管理 API
 */

/**
 * 检查指定应用的环境变量冲突
 * @param appType 应用类型（与 env_checker 后端的 get_keywords_for_app 对齐）
 * @returns 环境变量冲突列表
 */
export async function checkEnvConflicts(
  appType: string,
): Promise<EnvConflict[]> {
  return invoke<EnvConflict[]>("check_env_conflicts", { app: appType });
}

/**
 * 删除指定的环境变量 (会自动备份)
 * @param conflicts 要删除的环境变量冲突列表
 * @returns 备份信息
 */
export async function deleteEnvVars(
  conflicts: EnvConflict[],
): Promise<BackupInfo> {
  return invoke<BackupInfo>("delete_env_vars", { conflicts });
}

/**
 * 判断变量是否被 OGG 里配置的供应商正在使用
 * @returns 变量名(大写) -> 引用它的 "app · 供应商名" 列表
 */
export async function envVarsInUse(
  varNames: string[],
): Promise<Record<string, string[]>> {
  return invoke<Record<string, string[]>>("env_vars_in_use", { varNames });
}

/**
 * 列出可恢复的环境变量备份（最新在前）
 */
export async function listEnvBackups(): Promise<
  { backupPath: string; timestamp: string; varNames: string[] }[]
> {
  return invoke("list_env_backups");
}

/**
 * 读取单个备份（只读，不写入任何东西）
 */
export async function readEnvBackup(
  backupPath: string,
): Promise<BackupInfo> {
  return invoke<BackupInfo>("read_env_backup", { backupPath });
}

/**
 * 恢复前对比：给出每条变量的备份值 / 当前值 / 是否会被覆盖
 */
export async function diffEnvBackup(backupPath: string): Promise<EnvRestoreDiff> {
  return invoke<EnvRestoreDiff>("diff_env_backup", { backupPath });
}

/**
 * 从备份文件恢复环境变量
 * @param backupPath 备份文件路径
 */
export async function restoreEnvBackup(backupPath: string): Promise<void> {
  return invoke<void>("restore_env_backup", { backupPath });
}

/**
 * 检查所有应用的环境变量冲突
 * @returns 按应用类型分组的环境变量冲突
 *
 * app 列表 = OGG 实际管理环境变量语义的应用。历史上这里是写死的
 * claude/codex/gemini/grokbuild 四项——前三个来自已裁撤的运行时代码，
 * 只有它们的 env_checker 关键词还在。保留 claude/codex/gemini 是为了
 * 兼容用户手工遗留的 ANTHROPIC_/OPENAI_/GEMINI_ 变量，但加入 antigravity
 * （复用 GEMINI_ 关键词，受管键已在后端排除）。
 */
export async function checkAllEnvConflicts(): Promise<
  Record<string, EnvConflict[]>
> {
  const apps = ["claude", "codex", "gemini", "grokbuild", "antigravity"];
  const results: Record<string, EnvConflict[]> = {};

  await Promise.all(
    apps.map(async (app) => {
      try {
        results[app] = await checkEnvConflicts(app);
      } catch (error) {
        console.error(`检查 ${app} 环境变量失败:`, error);
        results[app] = [];
      }
    }),
  );

  return results;
}
