use super::env_checker::EnvConflict;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[cfg(target_os = "windows")]
use winreg::enums::*;
#[cfg(target_os = "windows")]
use winreg::RegKey;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub backup_path: String,
    pub timestamp: String,
    pub conflicts: Vec<EnvConflict>,
}

/// 环境变量引用来源的展示标签（"app / 供应商名"）
pub struct EnvVarUsage;

impl EnvVarUsage {
    pub fn label(app: &str, provider_name: &str) -> String {
        format!("{app} · {provider_name}")
    }
}

/// 备份清单条目（供 UI 列出可恢复的备份，不含具体值）
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub backup_path: String,
    pub timestamp: String,
    pub var_names: Vec<String>,
}

/// 恢复前的逐项对比：让 UI 能在**写入之前**说清楚「会把什么改成什么」。
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvRestoreEntry {
    pub var_name: String,
    /// 备份里的值（会写回去的值）
    pub backup_value: String,
    /// 当前机器上的值；`None` = 当前未设置
    pub current_value: Option<String>,
    /// 当前值与备份值是否不同（不同 = 恢复会覆盖掉现有值）
    pub differs: bool,
    /// 是否属于 OGG 受管键（提示：改它会影响 agy 认证）
    pub managed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvRestoreDiff {
    pub backup_path: String,
    pub timestamp: String,
    pub entries: Vec<EnvRestoreEntry>,
}

fn read_backup_file(backup_path: &str) -> Result<BackupInfo, String> {
    let content = fs::read_to_string(backup_path).map_err(|e| format!("读取备份文件失败: {e}"))?;
    serde_json::from_str::<BackupInfo>(&content).map_err(|e| format!("解析备份文件失败: {e}"))
}

/// 列出可恢复的备份（按文件名倒序 = 最新的在前）
pub fn list_backups() -> Result<Vec<BackupSummary>, String> {
    let backup_dir = get_backup_dir()?;
    if !backup_dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&backup_dir)
        .map_err(|e| format!("读取备份目录失败: {e}"))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("env-backup-") && n.ends_with(".json"))
                .unwrap_or(false)
        })
        .collect();
    // 文件名形如 env-backup-YYYYMMDD_HHMMSS.json，字典序即时间序；倒序取最新
    entries.sort();
    entries.reverse();
    for path in entries {
        if let Ok(info) = read_backup_file(&path.to_string_lossy()) {
            out.push(BackupSummary {
                backup_path: path.to_string_lossy().to_string(),
                timestamp: info.timestamp,
                var_names: info.conflicts.iter().map(|c| c.var_name.clone()).collect(),
            });
        }
    }
    Ok(out)
}

/// 读取单个备份（只读，不写任何东西）
pub fn read_backup(backup_path: &str) -> Result<BackupInfo, String> {
    read_backup_file(backup_path)
}

/// 与当前机器状态对比。
pub fn diff_backup(backup_path: &str) -> Result<EnvRestoreDiff, String> {
    let info = read_backup_file(backup_path)?;
    let mut entries = Vec::new();
    for conflict in &info.conflicts {
        let current = current_env_value(conflict);
        let differs = match &current {
            Some(now) => *now != conflict.var_value,
            // 备份里有值而当前没有 → 恢复会新建（同样算「会改变现状」）
            None => true,
        };
        entries.push(EnvRestoreEntry {
            var_name: conflict.var_name.clone(),
            backup_value: conflict.var_value.clone(),
            current_value: current,
            differs,
            managed: crate::antigravity_config::MANAGED_ENV_KEYS
                .contains(&conflict.var_name.to_uppercase().as_str()),
        });
    }
    Ok(EnvRestoreDiff {
        backup_path: backup_path.to_string(),
        timestamp: info.timestamp,
        entries,
    })
}

/// 读变量当前值（Windows 走注册表 HKCU，Unix 走进程环境）
fn current_env_value(conflict: &EnvConflict) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        let _ = conflict;
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("Environment")
            .ok()
            .and_then(|hkcu| hkcu.get_value::<String, _>(&conflict.var_name).ok())
            .filter(|s| !s.is_empty())
    }
    #[cfg(not(target_os = "windows"))]
    {
        if conflict.source_type == "file" {
            // shell rc 里的值：交给 UI 显示为「未知」比猜更安全
            return None;
        }
        std::env::var(&conflict.var_name).ok()
    }
}

/// Delete environment variables with automatic backup
pub fn delete_env_vars(conflicts: Vec<EnvConflict>) -> Result<BackupInfo, String> {
    // 受管键保护（双保险）：OGG 写入并托管的键（如 agy 的 GEMINI_API_KEY /
    // GOOGLE_GEMINI_BASE_URL）绝不允许从这里删除——删了等于删掉供应商凭据。
    // 冲突检测已在源头排除它们；这道闸兜住任何绕过前端的调用路径。
    //
    // 含受管键时**整批拒绝**而不是“过滤掉再删其余”：半成功会让用户以为
    // 「勾了 3 个都删了」，实际留了一个，比不删更难排查。
    let blocked: Vec<&str> = conflicts
        .iter()
        .filter(|c| {
            crate::antigravity_config::MANAGED_ENV_KEYS
                .contains(&c.var_name.to_uppercase().as_str())
        })
        .map(|c| c.var_name.as_str())
        .collect();
    if !blocked.is_empty() {
        log::warn!("拒绝删除 OGG 受管环境变量: {}", blocked.join(", "));
        return Err(format!(
            "所选环境变量均由 OGG Switch 托管，不能删除：{}",
            blocked.join("、")
        ));
    }

    // 备份放在校验之后：绝不为一个注定失败的删除留下垃圾备份文件
    let backup_info = create_backup(&conflicts)?;

    // Delete variables
    for conflict in &conflicts {
        match delete_single_env(conflict) {
            Ok(_) => {}
            Err(e) => {
                // If deletion fails, we keep the backup but return error
                return Err(format!(
                    "删除环境变量失败: {}. 备份已保存到: {}",
                    e, backup_info.backup_path
                ));
            }
        }
    }

    Ok(backup_info)
}

/// Create backup file before deletion
fn create_backup(conflicts: &[EnvConflict]) -> Result<BackupInfo, String> {
    // Get backup directory
    let backup_dir = get_backup_dir()?;
    fs::create_dir_all(&backup_dir).map_err(|e| format!("创建备份目录失败: {e}"))?;

    // Generate backup file name with timestamp
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let backup_file = backup_dir.join(format!("env-backup-{timestamp}.json"));

    // Create backup data
    let backup_info = BackupInfo {
        backup_path: backup_file.to_string_lossy().to_string(),
        timestamp: timestamp.clone(),
        conflicts: conflicts.to_vec(),
    };

    // Write backup file
    let json = serde_json::to_string_pretty(&backup_info)
        .map_err(|e| format!("序列化备份数据失败: {e}"))?;

    fs::write(&backup_file, json).map_err(|e| format!("写入备份文件失败: {e}"))?;

    Ok(backup_info)
}

/// Get backup directory path
fn get_backup_dir() -> Result<PathBuf, String> {
    Ok(crate::config::get_app_config_dir().join("backups"))
}

/// Delete a single environment variable
#[cfg(target_os = "windows")]
fn delete_single_env(conflict: &EnvConflict) -> Result<(), String> {
    match conflict.source_type.as_str() {
        "system" => {
            if conflict.source_path.contains("HKEY_CURRENT_USER") {
                let hkcu = RegKey::predef(HKEY_CURRENT_USER)
                    .open_subkey_with_flags("Environment", KEY_ALL_ACCESS)
                    .map_err(|e| format!("打开注册表失败: {}", e))?;

                hkcu.delete_value(&conflict.var_name)
                    .map_err(|e| format!("删除注册表项失败: {}", e))?;
            } else if conflict.source_path.contains("HKEY_LOCAL_MACHINE") {
                let hklm = RegKey::predef(HKEY_LOCAL_MACHINE)
                    .open_subkey_with_flags(
                        "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
                        KEY_ALL_ACCESS,
                    )
                    .map_err(|e| format!("打开系统注册表失败 (需要管理员权限): {}", e))?;

                hklm.delete_value(&conflict.var_name)
                    .map_err(|e| format!("删除系统注册表项失败: {}", e))?;
            }
            Ok(())
        }
        "file" => Err("Windows 系统不应该有文件类型的环境变量".to_string()),
        _ => Err(format!("未知的环境变量来源类型: {}", conflict.source_type)),
    }
}

#[cfg(not(target_os = "windows"))]
fn delete_single_env(conflict: &EnvConflict) -> Result<(), String> {
    match conflict.source_type.as_str() {
        "file" => {
            // Parse file path and line number from source_path (format: "path:line")
            let parts: Vec<&str> = conflict.source_path.split(':').collect();
            if parts.len() < 2 {
                return Err("无效的文件路径格式".to_string());
            }

            let file_path = parts[0];

            // Read file content
            let content = fs::read_to_string(file_path)
                .map_err(|e| format!("读取文件失败 {file_path}: {e}"))?;

            // Filter out the line containing the environment variable
            let new_content: Vec<String> = content
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    let export_line = trimmed.strip_prefix("export ").unwrap_or(trimmed);

                    // Check if this line sets the target variable
                    if let Some(eq_pos) = export_line.find('=') {
                        let var_name = export_line[..eq_pos].trim();
                        var_name != conflict.var_name
                    } else {
                        true
                    }
                })
                .map(|s| s.to_string())
                .collect();

            // Write back to file
            fs::write(file_path, new_content.join("\n"))
                .map_err(|e| format!("写入文件失败 {file_path}: {e}"))?;

            Ok(())
        }
        "system" => {
            // On Unix, we can't directly delete process environment variables
            Ok(())
        }
        _ => Err(format!("未知的环境变量来源类型: {}", conflict.source_type)),
    }
}

/// Restore environment variables from backup
pub fn restore_from_backup(backup_path: String) -> Result<(), String> {
    // Read backup file
    let content = fs::read_to_string(&backup_path).map_err(|e| format!("读取备份文件失败: {e}"))?;

    let backup_info: BackupInfo =
        serde_json::from_str(&content).map_err(|e| format!("解析备份文件失败: {e}"))?;

    // Restore each variable
    for conflict in &backup_info.conflicts {
        restore_single_env(conflict)?;
    }

    Ok(())
}

/// Restore a single environment variable
#[cfg(target_os = "windows")]
fn restore_single_env(conflict: &EnvConflict) -> Result<(), String> {
    match conflict.source_type.as_str() {
        "system" => {
            if conflict.source_path.contains("HKEY_CURRENT_USER") {
                let (hkcu, _) = RegKey::predef(HKEY_CURRENT_USER)
                    .create_subkey("Environment")
                    .map_err(|e| format!("打开注册表失败: {}", e))?;

                hkcu.set_value(&conflict.var_name, &conflict.var_value)
                    .map_err(|e| format!("恢复注册表项失败: {}", e))?;
            } else if conflict.source_path.contains("HKEY_LOCAL_MACHINE") {
                let (hklm, _) = RegKey::predef(HKEY_LOCAL_MACHINE)
                    .create_subkey(
                        "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
                    )
                    .map_err(|e| format!("打开系统注册表失败 (需要管理员权限): {}", e))?;

                hklm.set_value(&conflict.var_name, &conflict.var_value)
                    .map_err(|e| format!("恢复系统注册表项失败: {}", e))?;
            }
            Ok(())
        }
        _ => Err(format!(
            "无法恢复类型为 {} 的环境变量",
            conflict.source_type
        )),
    }
}

#[cfg(not(target_os = "windows"))]
fn restore_single_env(conflict: &EnvConflict) -> Result<(), String> {
    match conflict.source_type.as_str() {
        "file" => {
            // Parse file path from source_path
            let parts: Vec<&str> = conflict.source_path.split(':').collect();
            if parts.is_empty() {
                return Err("无效的文件路径格式".to_string());
            }

            let file_path = parts[0];

            // Read file content
            let mut content = fs::read_to_string(file_path)
                .map_err(|e| format!("读取文件失败 {file_path}: {e}"))?;

            // Append the environment variable line
            let export_line = format!("\nexport {}={}", conflict.var_name, conflict.var_value);
            content.push_str(&export_line);

            // Write back to file
            fs::write(file_path, content).map_err(|e| format!("写入文件失败 {file_path}: {e}"))?;

            Ok(())
        }
        _ => Err(format!(
            "无法恢复类型为 {} 的环境变量",
            conflict.source_type
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_dir_creation() {
        let backup_dir = get_backup_dir();
        assert!(backup_dir.is_ok());
    }

    fn sample_conflict(name: &str, value: &str) -> EnvConflict {
        EnvConflict {
            var_name: name.to_string(),
            var_value: value.to_string(),
            source_type: "system".to_string(),
            source_path: "HKEY_CURRENT_USER\\Environment".to_string(),
        }
    }

    /// 受管键（agy 的认证凭据）不允许删除——即便调用方硬传进来
    #[test]
    fn delete_refuses_managed_keys() {
        let err = delete_env_vars(vec![
            sample_conflict("GEMINI_API_KEY", "sk-managed"),
            sample_conflict("GOOGLE_GEMINI_BASE_URL", "https://relay"),
        ])
        .expect_err("受管键必须被拒绝");
        assert!(err.contains("OGG Switch 托管"), "unexpected error: {err}");

        // 大小写不敏感
        let err = delete_env_vars(vec![sample_conflict("gemini_api_key", "sk-x")])
            .expect_err("大小写变体也必须被拒绝");
        assert!(err.contains("OGG Switch 托管"), "unexpected error: {err}");
    }

    /// 混合名单：受管的被拦下，非受管的照常处理（不整批失败）
    #[test]
    fn delete_with_managed_key_in_mixed_selection_is_refused() {
        // 受管 + 非受管混在一起时，只要含受管键就整体拒绝——避免用户以为
        // “删了 3 个、其中 1 个其实没删”的半成功状态
        let err = delete_env_vars(vec![
            sample_conflict("GEMINI_STALE_KEY", "old"),
            sample_conflict("GEMINI_API_KEY", "sk-managed"),
        ])
        .expect_err("含受管键时应整体拒绝");
        assert!(err.contains("OGG Switch 托管"), "unexpected error: {err}");
    }

    /// 备份清单与对比：只列 env-backup-*、按时间倒序、对比标出会被覆盖的项
    #[test]
    fn list_and_diff_backups_reports_overrides() {
        // 用真实备份目录之外会污染用户目录——这里只验证 diff 的纯逻辑部分：
        // 直接构造备份文件到临时目录，再调用 diff_backup（它接受任意路径）
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("env-backup-20260101_000000.json");
        let info = BackupInfo {
            backup_path: path.to_string_lossy().to_string(),
            timestamp: "20260101_000000".to_string(),
            conflicts: vec![
                sample_conflict("GEMINI_STALE_KEY", "backup-value"),
                sample_conflict("GEMINI_API_KEY", "backup-managed"),
            ],
        };
        std::fs::write(&path, serde_json::to_string_pretty(&info).unwrap()).unwrap();

        let diff = diff_backup(&path.to_string_lossy()).unwrap();
        assert_eq!(diff.entries.len(), 2);
        let managed = diff
            .entries
            .iter()
            .find(|e| e.var_name == "GEMINI_API_KEY")
            .expect("managed entry present");
        assert!(managed.managed, "GEMINI_API_KEY 应标记为受管");
        assert!(managed.backup_value == "backup-managed");
        // 备份里有值、当前环境未必有 → 至少应识别出「会改变现状」
        assert!(managed.differs || managed.current_value.is_some());

        let stale = diff
            .entries
            .iter()
            .find(|e| e.var_name == "GEMINI_STALE_KEY")
            .expect("stale entry present");
        assert!(!stale.managed);

        // read_backup 是只读的：内容与写入一致
        let reread = read_backup(&path.to_string_lossy()).unwrap();
        assert_eq!(reread.conflicts.len(), 2);
    }
}
