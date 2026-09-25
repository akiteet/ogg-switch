//! CLI 官方登录态的备份 / 恢复。
//!
//! **背景**：切到第三方供应商时，Grok 与 Codex 的 live 写入会**故意删除**各自的
//! `auth.json`——防止官方 OAuth token 被发往第三方 base_url，触发 401 → 自动 OIDC 登录
//! → 再 401 的死循环（见 `grok_config::write_grok_provider_live` 的 issue #6589 注释、
//! `codex_config::remove_codex_live_auth_after_third_party_switch`）。删除本身是对的，
//! 缺的是**切回官方时的恢复路径**：文件没了就是没了，用户只能重新登录
//! （2026-09-24 报障「每次从第三方切回官方供应商都要重新登录」）。
//!
//! 这里补上"可恢复"：删除**前**把文件复制到 OGG 自己的备份目录，切回官方供应商时若本机
//! 已缺失就复制回去。
//!
//! 两条刻意的不变式：
//! - **本机现有文件永远优先**：只在目标不存在时恢复。用户可能刚重新登录过，那份更新。
//! - **失败不阻断切换**：备份/恢复失败只记日志，切换照常完成（与既有的 auth 清理同策略）。

use crate::error::AppError;
use std::path::{Path, PathBuf};

/// 备份目录：`<app 数据目录>/backups/cli-auth/`（与 env 备份同处 `backups/` 下，单独子目录）
pub fn backup_dir() -> PathBuf {
    crate::config::get_app_config_dir()
        .join("backups")
        .join("cli-auth")
}

/// `app` 的登录文件备份路径，例如 `grokbuild-auth.json`。
pub fn backup_path(app: &str) -> PathBuf {
    backup_dir().join(format!("{app}-auth.json"))
}

/// 备份一份 CLI 官方登录文件。源不存在 → `Ok(false)`（没什么可备份）。
/// 返回是否真的写出了备份。
pub fn backup_cli_auth(app: &str, source: &Path) -> Result<bool, AppError> {
    if !source.is_file() {
        return Ok(false);
    }
    let contents = std::fs::read(source).map_err(|err| AppError::io(source, err))?;
    let target = backup_path(app);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|err| AppError::io(parent, err))?;
    }
    write_private(&target, &contents)?;
    log::info!(
        "已备份 {} 官方登录态 -> {}（{} 字节）",
        app,
        target.display(),
        contents.len()
    );
    Ok(true)
}

/// 目标缺失且备份存在时恢复。目标已存在 → `Ok(false)`（本机新登录态优先）。
pub fn restore_cli_auth_if_missing(app: &str, target: &Path) -> Result<bool, AppError> {
    if target.exists() {
        return Ok(false);
    }
    let backup = backup_path(app);
    if !backup.is_file() {
        return Ok(false);
    }
    let contents = std::fs::read(&backup).map_err(|err| AppError::io(&backup, err))?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|err| AppError::io(parent, err))?;
    }
    write_private(target, &contents)?;
    log::info!(
        "已从备份恢复 {} 官方登录态 -> {}（无需重新登录）",
        app,
        target.display()
    );
    Ok(true)
}

/// 写入文件；Unix 下收权限到 0600（凭据副本不该对同机其它用户可读）。
fn write_private(path: &Path, contents: &[u8]) -> Result<(), AppError> {
    std::fs::write(path, contents).map_err(|err| AppError::io(path, err))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(err) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            log::warn!("收紧 {} 权限失败: {err}", path.display());
        }
    }
    Ok(())
}

/// 切换流程用的便捷包装：失败只告警，绝不阻断切换（返回是否成功备份/恢复）。
pub fn backup_cli_auth_logging(app: &str, source: &Path) -> bool {
    match backup_cli_auth(app, source) {
        Ok(done) => done,
        Err(err) => {
            log::warn!("备份 {} 官方登录态失败（不阻断切换）: {err}", app);
            false
        }
    }
}

/// 同 [`backup_cli_auth_logging`]，用于恢复路径。
pub fn restore_cli_auth_logging(app: &str, target: &Path) -> bool {
    match restore_cli_auth_if_missing(app, target) {
        Ok(done) => done,
        Err(err) => {
            log::warn!("恢复 {} 官方登录态失败（不阻断切换）: {err}", app);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 隔离的临时数据目录：`get_app_config_dir()` 支持 override（测试环境变量在
    /// `config.rs` 里统一处理），这里直接切 `CC_SWITCH_TEST_HOME`。
    fn with_test_home<T>(test_fn: impl FnOnce(&Path) -> T) -> T {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        let _guard = LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|err| err.into_inner());

        let tmp = tempfile::tempdir().unwrap();
        let old = std::env::var_os("CC_SWITCH_TEST_HOME");
        std::env::set_var("CC_SWITCH_TEST_HOME", tmp.path());
        let result = test_fn(tmp.path());
        match old {
            Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
            None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
        }
        result
    }

    fn write_auth(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn backup_then_restore_round_trips_the_login_file() {
        with_test_home(|home| {
            let auth = home.join(".grok").join("auth.json");
            write_auth(&auth, r#"{"token":"official-login"}"#);

            assert!(backup_cli_auth("grokbuild", &auth).unwrap());
            assert!(backup_path("grokbuild").is_file());

            // 切到第三方：文件被删
            std::fs::remove_file(&auth).unwrap();

            // 切回官方：恢复
            assert!(restore_cli_auth_if_missing("grokbuild", &auth).unwrap());
            assert_eq!(
                std::fs::read_to_string(&auth).unwrap(),
                r#"{"token":"official-login"}"#
            );
        });
    }

    #[test]
    fn restore_never_overwrites_a_fresh_login() {
        with_test_home(|home| {
            let auth = home.join(".codex").join("auth.json");
            write_auth(&auth, r#"{"token":"old-login"}"#);
            backup_cli_auth("codex", &auth).unwrap();

            // 用户在第三方期间重新登录过 → 本机文件是新的
            write_auth(&auth, r#"{"token":"fresh-login"}"#);
            assert!(!restore_cli_auth_if_missing("codex", &auth).unwrap());
            assert_eq!(
                std::fs::read_to_string(&auth).unwrap(),
                r#"{"token":"fresh-login"}"#
            );
        });
    }

    #[test]
    fn missing_source_or_backup_is_a_no_op() {
        with_test_home(|home| {
            let auth = home.join(".grok").join("auth.json");

            // 没有源文件 → 不产生备份
            assert!(!backup_cli_auth("grokbuild", &auth).unwrap());
            assert!(!backup_path("grokbuild").exists());

            // 没有备份 → 不创建目标
            assert!(!restore_cli_auth_if_missing("grokbuild", &auth).unwrap());
            assert!(!auth.exists());
        });
    }

    #[test]
    fn backup_refreshes_when_source_is_written_again() {
        with_test_home(|home| {
            let auth = home.join(".grok").join("auth.json");
            write_auth(&auth, "first");
            backup_cli_auth("grokbuild", &auth).unwrap();

            // 重新登录后再切第三方：备份应被最新内容刷新
            write_auth(&auth, "second");
            backup_cli_auth("grokbuild", &auth).unwrap();
            std::fs::remove_file(&auth).unwrap();
            restore_cli_auth_if_missing("grokbuild", &auth).unwrap();

            assert_eq!(std::fs::read_to_string(&auth).unwrap(), "second");
        });
    }

    #[cfg(unix)]
    #[test]
    fn backup_file_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;

        with_test_home(|home| {
            let auth = home.join(".grok").join("auth.json");
            write_auth(&auth, "token");
            backup_cli_auth("grokbuild", &auth).unwrap();

            let mode = std::fs::metadata(backup_path("grokbuild"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "mode={mode:o}");
        });
    }
}
