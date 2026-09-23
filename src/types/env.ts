/**
 * 环境变量冲突检测相关类型定义
 */

/**
 * 环境变量冲突信息
 */
export interface EnvConflict {
  /** 环境变量名称 */
  varName: string;
  /** 环境变量的值 */
  varValue: string;
  /** 来源类型: "system" 表示系统环境变量, "file" 表示配置文件 */
  sourceType: "system" | "file";
  /** 来源路径 (注册表路径或文件路径:行号) */
  sourcePath: string;
}

/**
 * 备份信息
 */
export interface BackupInfo {
  /** 备份文件路径 */
  backupPath: string;
  /** 备份时间戳 */
  timestamp: string;
  /** 被备份的环境变量冲突列表 */
  conflicts: EnvConflict[];
}

/**
 * 恢复前的逐项对比结果（先看清「会把什么改成什么」再写入）
 */
export interface EnvRestoreEntry {
  varName: string;
  /** 备份里的值（恢复后会写入的值） */
  backupValue: string;
  /** 当前机器上的值；undefined = 当前未设置 */
  currentValue?: string;
  /** 当前值与备份值不同（恢复会覆盖现有值） */
  differs: boolean;
  /** 是否属于 OGG 受管键（影响 agy 认证） */
  managed: boolean;
}

export interface EnvRestoreDiff {
  backupPath: string;
  timestamp: string;
  entries: EnvRestoreEntry[];
}

/** 备份清单条目（不含具体值） */
export interface EnvBackupSummary {
  backupPath: string;
  timestamp: string;
  varNames: string[];
}
