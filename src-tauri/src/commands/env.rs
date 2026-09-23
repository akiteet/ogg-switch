use crate::services::env_checker::{check_env_conflicts as check_conflicts, EnvConflict};
use crate::services::env_manager::{
    delete_env_vars as delete_vars, list_backups, read_backup, restore_from_backup, BackupInfo,
    BackupSummary, EnvRestoreDiff,
};

/// 判断一组环境变量是否被 OGG 里配置的供应商**正在使用**。
///
/// 删除前的关键信号：一个变量即使不是 OGG 受管键，也可能正是某供应商条目里存的
/// 凭据/端点。此时删掉它，供应商卡片还在但会失效——用户无从察觉。
///
/// 判定方式：逐 app 读供应商表，凡 `settingsConfig.env` 里存在同名键且值与待删值
/// 相同的，计为一次引用。注意只比对「当前生效槽位」（持久环境变量语义下同一时刻
/// 只有一个值在用），因此不做跨值匹配。
#[tauri::command]
pub fn env_vars_in_use(
    state: tauri::State<'_, crate::store::AppState>,
    var_names: Vec<String>,
) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
    use crate::services::env_manager::EnvVarUsage;
    let mut out: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let wanted: Vec<String> = var_names.iter().map(|n| n.to_uppercase()).collect();
    if wanted.is_empty() {
        return Ok(out);
    }

    for app in ["grokbuild", "antigravity", "omp"] {
        let providers = match state.db.get_all_providers(app) {
            Ok(p) => p,
            Err(_) => continue,
        };
        for provider in providers.values() {
            let env = provider
                .settings_config
                .get("env")
                .and_then(serde_json::Value::as_object);
            let Some(env) = env else { continue };
            for (key, value) in env {
                let Some(value) = value.as_str() else { continue };
                if value.trim().is_empty() {
                    continue;
                }
                let upper = key.to_uppercase();
                // 只关心待删除列表里的键
                if !wanted.iter().any(|w| *w == upper) {
                    continue;
                }
                out.entry(upper.clone())
                    .or_default()
                    .push(EnvVarUsage::label(app, &provider.name));
            }
        }
    }
    Ok(out)
}

/// Check environment variable conflicts for a specific app
#[tauri::command]
pub fn check_env_conflicts(app: String) -> Result<Vec<EnvConflict>, String> {
    check_conflicts(&app)
}

/// Delete environment variables with backup
#[tauri::command]
pub fn delete_env_vars(conflicts: Vec<EnvConflict>) -> Result<BackupInfo, String> {
    delete_vars(conflicts)
}

/// List recoverable environment variable backups (newest first)
#[tauri::command]
pub fn list_env_backups() -> Result<Vec<BackupSummary>, String> {
    list_backups()
}

/// Read one backup file for a "before restoring" review (no writes)
#[tauri::command]
pub fn read_env_backup(backup_path: String) -> Result<BackupInfo, String> {
    read_backup(&backup_path)
}

/// Compare a backup against the current machine state: per variable, whether it is
/// currently set, whether the value differs, and whether an OGG-managed provider
/// would be affected. Restoring overwrites unconditionally — this exists so the UI
/// can show that **before** anything is written.
#[tauri::command]
pub fn diff_env_backup(backup_path: String) -> Result<EnvRestoreDiff, String> {
    crate::services::env_manager::diff_backup(&backup_path)
}

/// Restore environment variables from backup file
#[tauri::command]
pub fn restore_env_backup(backup_path: String) -> Result<(), String> {
    restore_from_backup(backup_path)
}
