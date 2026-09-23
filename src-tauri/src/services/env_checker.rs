use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvConflict {
    pub var_name: String,
    pub var_value: String,
    pub source_type: String, // "system" | "file"
    pub source_path: String, // Registry path or file path
}

#[cfg(target_os = "windows")]
use winreg::enums::*;
#[cfg(target_os = "windows")]
use winreg::RegKey;

/// Check environment variables for conflicts
pub fn check_env_conflicts(app: &str) -> Result<Vec<EnvConflict>, String> {
    let keywords = get_keywords_for_app(app);
    let mut conflicts = Vec::new();

    // Check system environment variables
    conflicts.extend(check_system_env(&keywords)?);

    // Check shell configuration files (Unix only)
    #[cfg(not(target_os = "windows"))]
    conflicts.extend(check_shell_configs(&keywords)?);

    Ok(conflicts)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvKeyword {
    Exact(&'static str),
    Prefix(&'static str),
}

/// Get relevant keywords for each app
fn get_keywords_for_app(app: &str) -> Vec<EnvKeyword> {
    match app.to_lowercase().as_str() {
        "claude" => vec![EnvKeyword::Prefix("ANTHROPIC")],
        "codex" => vec![EnvKeyword::Prefix("OPENAI")],
        "gemini" => vec![
            EnvKeyword::Prefix("GEMINI"),
            EnvKeyword::Prefix("GOOGLE_GEMINI"),
        ],
        "grokbuild" | "grok" => vec![
            EnvKeyword::Exact("XAI_API_KEY"),
            EnvKeyword::Exact("GROK_DEFAULT_MODEL"),
        ],
        _ => vec![],
    }
}

fn matches_env_keyword(name: &str, keywords: &[EnvKeyword]) -> bool {
    let upper_name = name.to_uppercase();
    keywords.iter().any(|keyword| match keyword {
        EnvKeyword::Exact(name) => upper_name == *name,
        EnvKeyword::Prefix(prefix) => upper_name.starts_with(prefix),
    })
}

/// OGG Switch 自己写入并托管的持久环境变量。
///
/// 这些键出现在注册表 / shell rc 里是**正常工作状态**（切换供应商时由 OGG 写入，
/// agy 等工具靠它们认证），不是「覆盖用户配置的冲突」——把它们列进冲突清单并
/// 提供一键删除，等于引导用户删掉自己供应商的凭据（实际发生过）。因此检测时
/// 一律排除。
///
/// 与 `antigravity_config::MANAGED_ENV_KEYS` 保持同一份事实：agy 不读 `~/.gemini/.env`、
/// 没有其他配置入口，这两个键就是它的认证方式。
fn managed_env_keys() -> [&'static str; 2] {
    crate::antigravity_config::MANAGED_ENV_KEYS
}

fn is_managed_env_key(name: &str) -> bool {
    let upper = name.to_uppercase();
    managed_env_keys().iter().any(|key| upper == *key)
}

/// Check system environment variables (Windows Registry or Unix env)
#[cfg(target_os = "windows")]
fn check_system_env(keywords: &[EnvKeyword]) -> Result<Vec<EnvConflict>, String> {
    let mut conflicts = Vec::new();

    // Check HKEY_CURRENT_USER\Environment
    if let Ok(hkcu) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Environment") {
        for (name, value) in hkcu.enum_values().filter_map(Result::ok) {
            if matches_env_keyword(&name, keywords) && !is_managed_env_key(&name) {
                conflicts.push(EnvConflict {
                    var_name: name.clone(),
                    var_value: value.to_string(),
                    source_type: "system".to_string(),
                    source_path: "HKEY_CURRENT_USER\\Environment".to_string(),
                });
            }
        }
    }

    // Check HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Session Manager\Environment
    if let Ok(hklm) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
    {
        for (name, value) in hklm.enum_values().filter_map(Result::ok) {
            if matches_env_keyword(&name, keywords) && !is_managed_env_key(&name) {
                conflicts.push(EnvConflict {
                    var_name: name.clone(),
                    var_value: value.to_string(),
                    source_type: "system".to_string(),
                    source_path: "HKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment".to_string(),
                });
            }
        }
    }

    Ok(conflicts)
}

#[cfg(not(target_os = "windows"))]
fn check_system_env(keywords: &[EnvKeyword]) -> Result<Vec<EnvConflict>, String> {
    let mut conflicts = Vec::new();

    // Check current process environment
    for (key, value) in std::env::vars() {
        if matches_env_keyword(&key, keywords) && !is_managed_env_key(&key) {
            conflicts.push(EnvConflict {
                var_name: key,
                var_value: value,
                source_type: "system".to_string(),
                source_path: "Process Environment".to_string(),
            });
        }
    }

    Ok(conflicts)
}

/// Check shell configuration files for environment variable exports（仅 Unix 会调用）。
///
/// **刻意不在函数上做 cfg 门控**：`not(windows)` 专属代码若只在 Linux/macOS 编译，
/// 本机（Windows）永远验不到它——v1.1.0 就因为一个只在非 Windows 编译的模块缺
/// `use std::fs;` 而在 CI 上翻车。这里与 `antigravity_config::rc_block` 采取同一手法：
/// 跨平台编译（Windows 上允许 dead_code），让单测能在任意平台执行到它。
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn check_shell_configs(keywords: &[EnvKeyword]) -> Result<Vec<EnvConflict>, String> {
    let mut conflicts = Vec::new();

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let config_files = vec![
        format!("{}/.bashrc", home),
        format!("{}/.bash_profile", home),
        format!("{}/.zshrc", home),
        format!("{}/.zprofile", home),
        format!("{}/.profile", home),
        "/etc/profile".to_string(),
        "/etc/bashrc".to_string(),
    ];

    for file_path in config_files {
        if let Ok(content) = fs::read_to_string(&file_path) {
            // OGG 的受管块（`# >>> OGG-Switch (antigravity) >>>` … `# <<<`）里的键
            // 是 OGG 自己写的，同样不是冲突。
            let mut inside_managed_block = false;
            // Parse lines for export statements
            for (line_num, line) in content.lines().enumerate() {
                let trimmed = line.trim();

                if trimmed.starts_with(crate::antigravity_config::rc_block::BLOCK_BEGIN) {
                    inside_managed_block = true;
                    continue;
                }
                if trimmed.starts_with(crate::antigravity_config::rc_block::BLOCK_END) {
                    inside_managed_block = false;
                    continue;
                }
                if inside_managed_block {
                    continue;
                }

                // Match patterns like: export VAR=value or VAR=value
                if trimmed.starts_with("export ")
                    || (!trimmed.starts_with('#') && trimmed.contains('='))
                {
                    let export_line = trimmed.strip_prefix("export ").unwrap_or(trimmed);

                    if let Some(eq_pos) = export_line.find('=') {
                        let var_name = export_line[..eq_pos].trim();
                        let var_value = export_line[eq_pos + 1..].trim();

                        // Check if variable name contains any keyword
                        if matches_env_keyword(var_name, keywords) && !is_managed_env_key(var_name)
                        {
                            conflicts.push(EnvConflict {
                                var_name: var_name.to_string(),
                                var_value: var_value
                                    .trim_matches('"')
                                    .trim_matches('\'')
                                    .to_string(),
                                source_type: "file".to_string(),
                                source_path: format!("{}:{}", file_path, line_num + 1),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(conflicts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_keywords() {
        assert_eq!(
            get_keywords_for_app("claude"),
            vec![EnvKeyword::Prefix("ANTHROPIC")]
        );
        assert_eq!(
            get_keywords_for_app("codex"),
            vec![EnvKeyword::Prefix("OPENAI")]
        );
        assert_eq!(
            get_keywords_for_app("gemini"),
            vec![
                EnvKeyword::Prefix("GEMINI"),
                EnvKeyword::Prefix("GOOGLE_GEMINI")
            ]
        );
        assert_eq!(
            get_keywords_for_app("grokbuild"),
            vec![
                EnvKeyword::Exact("XAI_API_KEY"),
                EnvKeyword::Exact("GROK_DEFAULT_MODEL")
            ]
        );
        assert_eq!(
            get_keywords_for_app("grok"),
            get_keywords_for_app("grokbuild")
        );
        assert_eq!(get_keywords_for_app("unknown"), Vec::<EnvKeyword>::new());
    }

    #[test]
    fn grok_keywords_only_match_credentials() {
        let keywords = get_keywords_for_app("grokbuild");

        assert!(matches_env_keyword("XAI_API_KEY", &keywords));
        assert!(matches_env_keyword("xai_api_key", &keywords));
        assert!(matches_env_keyword("GROK_DEFAULT_MODEL", &keywords));
        assert!(matches_env_keyword("grok_default_model", &keywords));
        assert!(!matches_env_keyword("MY_XAI_API_KEY", &keywords));
        assert!(!matches_env_keyword("XAI_API_KEY_BACKUP", &keywords));
        assert!(!matches_env_keyword("MY_GROK_DEFAULT_MODEL", &keywords));
        assert!(!matches_env_keyword("GROK_DEFAULT_MODEL_BACKUP", &keywords));
        assert!(!matches_env_keyword("GROK_BIN_DIR", &keywords));
        assert!(!matches_env_keyword("GROK_HOME", &keywords));
    }

    #[test]
    fn broad_app_keywords_match_only_at_the_start() {
        let keywords = get_keywords_for_app("claude");

        assert!(matches_env_keyword("ANTHROPIC_API_KEY", &keywords));
        assert!(matches_env_keyword("anthropic_base_url", &keywords));
        assert!(!matches_env_keyword("MY_ANTHROPIC_API_KEY", &keywords));
        assert!(!matches_env_keyword("NOT_ANTHROPIC", &keywords));
    }

    #[test]
    fn managed_keys_are_never_reported_even_when_matching() {
        // GEMINI_API_KEY / GOOGLE_GEMINI_BASE_URL 都命中 gemini 的前缀关键词，
        // 但它们是 OGG 写入并托管的（agy 靠它们认证）——绝不能进冲突清单
        assert!(is_managed_env_key("GEMINI_API_KEY"));
        assert!(is_managed_env_key("gemini_api_key"));
        assert!(is_managed_env_key("GOOGLE_GEMINI_BASE_URL"));
        assert!(is_managed_env_key("google_gemini_base_url"));

        let keywords = get_keywords_for_app("gemini");
        assert!(matches_env_keyword("GEMINI_API_KEY", &keywords));
        assert!(matches_env_keyword("GOOGLE_GEMINI_BASE_URL", &keywords));

        // 非受管的 GEMINI_ 前缀键仍然要报（可能是用户自己设的旧值）
        assert!(!is_managed_env_key("GEMINI_API_KEY_OLD"));
        assert!(!is_managed_env_key("GOOGLE_GEMINI_BASE_URL_BACKUP"));
        assert!(!is_managed_env_key("GEMINI_MODEL"));
        assert!(!is_managed_env_key("GOOGLE_API_KEY"));
    }

    /// 系统侧检测端到端：受管键被排除、非受管键保留。
    /// 走进程环境注入（POSIX 分支）；Windows 注册表分支无法在单测里安全注入，
    /// 由同一 `is_managed_env_key` 过滤保证行为一致。
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn system_env_check_skips_managed_keys() {
        let keywords = get_keywords_for_app("gemini");
        std::env::set_var("GEMINI_API_KEY", "ogg-managed");
        std::env::set_var("GEMINI_STALE_KEY", "user-set");

        let conflicts = check_system_env(&keywords).unwrap();
        let names: Vec<&str> = conflicts.iter().map(|c| c.var_name.as_str()).collect();
        assert!(!names.contains(&"GEMINI_API_KEY"), "{names:?}");
        assert!(names.contains(&"GEMINI_STALE_KEY"), "{names:?}");

        std::env::remove_var("GEMINI_API_KEY");
        std::env::remove_var("GEMINI_STALE_KEY");
    }

    /// shell rc 检测端到端：受管块内的键排除，块外同名的用户 export 保留。
    ///
    /// 刻意不在 Windows 上跳过：这条测试覆盖的正是本轮新增的受管块跳过逻辑，
    /// 只在 Linux/macOS 跑就等于本地永远验不到它。会改 HOME，故加 serial。
    #[test]
    #[serial_test::serial]
    fn shell_config_check_skips_managed_block() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join(".bashrc");
        std::fs::write(
            &rc,
            format!(
                concat!(
                    "{}\n",
                    "export GEMINI_API_KEY='ogg-managed'\n",
                    "{}\n",
                    "export GEMINI_STALE_KEY='user-set'\n"
                ),
                crate::antigravity_config::rc_block::BLOCK_BEGIN,
                crate::antigravity_config::rc_block::BLOCK_END
            ),
        )
        .unwrap();

        // 把 HOME 指到临时目录（check_shell_configs 读 $HOME）
        let old_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", dir.path());
        let result = check_shell_configs(&get_keywords_for_app("gemini"));
        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let conflicts = result.unwrap();

        let names: Vec<&str> = conflicts.iter().map(|c| c.var_name.as_str()).collect();
        assert!(!names.contains(&"GEMINI_API_KEY"), "{names:?}");
        assert!(names.contains(&"GEMINI_STALE_KEY"), "{names:?}");
    }
}
