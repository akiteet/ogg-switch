use serde_json::json;
use std::fs;
use std::path::PathBuf;

use ogg_switch_lib::{
    get_claude_settings_path, read_json_file, AppError, AppType, ConfigService, MultiAppConfig,
    Provider, ProviderMeta,
};

#[path = "support.rs"]
mod support;
use support::{
    create_test_state, create_test_state_with_config, enable_codex_official_auth_preservation,
    ensure_test_home, reset_test_fs, test_mutex,
};

#[test]
fn sync_claude_provider_writes_live_settings() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let mut config = MultiAppConfig::default();
    let provider_config = json!({
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "test-key",
            "ANTHROPIC_BASE_URL": "https://api.test"
        },
        "ui": {
            "displayName": "Test Provider"
        }
    });

    let provider = Provider::with_id(
        "prov-1".to_string(),
        "Test Claude".to_string(),
        provider_config.clone(),
        None,
    );

    let manager = config
        .get_manager_mut(&AppType::Claude)
        .expect("claude manager");
    manager.providers.insert("prov-1".to_string(), provider);
    manager.current = "prov-1".to_string();

    ConfigService::sync_current_providers_to_live(&mut config).expect("sync live settings");

    let settings_path = get_claude_settings_path();
    assert!(
        settings_path.exists(),
        "live settings should be written to {}",
        settings_path.display()
    );

    let live_value: serde_json::Value = read_json_file(&settings_path).expect("read live file");
    assert_eq!(live_value, provider_config);

    // 确认 SSOT 中的供应商也同步了最新内容
    let updated = config
        .get_manager(&AppType::Claude)
        .and_then(|m| m.providers.get("prov-1"))
        .expect("provider in config");
    assert_eq!(updated.settings_config, provider_config);

    // 额外确认写入位置位于测试 HOME 下
    assert!(
        settings_path.starts_with(home),
        "settings path {settings_path:?} should reside under test HOME {home:?}"
    );
}

#[test]
fn sync_codex_provider_writes_config_without_touching_auth() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    enable_codex_official_auth_preservation();

    let mut config = MultiAppConfig::default();

    // 注意：v3.7.0 后 MCP 同步由 McpService 独立处理，不再通过 provider 切换触发
    // Codex provider 切换只写 config.toml；auth.json 保留用户登录态。

    let provider_config = json!({
        "auth": {
            "OPENAI_API_KEY": "codex-key"
        },
        "config": r#"base_url = "https://codex.test""#
    });

    let provider = Provider::with_id(
        "codex-1".to_string(),
        "Codex Test".to_string(),
        provider_config.clone(),
        None,
    );

    let manager = config
        .get_manager_mut(&AppType::Codex)
        .expect("codex manager");
    manager.providers.insert("codex-1".to_string(), provider);
    manager.current = "codex-1".to_string();

    ConfigService::sync_current_providers_to_live(&mut config).expect("sync codex live");

    let auth_path = ogg_switch_lib::get_codex_auth_path();
    let config_path = ogg_switch_lib::get_codex_config_path();

    assert!(
        !auth_path.exists(),
        "auth.json should not be created by provider switching at {}",
        auth_path.display()
    );
    assert!(
        config_path.exists(),
        "config.toml should exist at {}",
        config_path.display()
    );

    let toml_text = fs::read_to_string(&config_path).expect("read config.toml");
    assert!(
        toml_text.contains("base_url"),
        "config.toml should contain base_url from provider config"
    );
    assert!(
        toml_text.contains("experimental_bearer_token"),
        "config.toml should contain provider-scoped bearer token"
    );

    let manager = config.get_manager(&AppType::Codex).expect("codex manager");
    let synced = manager.providers.get("codex-1").expect("codex provider");
    let synced_cfg = synced
        .settings_config
        .get("config")
        .and_then(|v| v.as_str())
        .expect("config string");
    assert!(
        !synced_cfg.contains("experimental_bearer_token"),
        "provider storage should not persist generated live bearer token"
    );
    assert!(
        toml_text.contains("experimental_bearer_token"),
        "live config should include generated bearer token"
    );
}

#[test]
fn sync_codex_provider_with_config_only_token_backfills_auth() {
    // P2-2 回归: stored provider 的 token 只藏在 config.toml 的 experimental_bearer_token 时,
    // sync 路径必须把 token 从 live config 提取并写回 stored auth.OPENAI_API_KEY,
    // 否则下一轮 sync 会在 cleaned config + 空 auth 之间丢失 token。
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();

    let mut config = MultiAppConfig::default();

    let stored_config = r#"model_provider = "thirdparty"
model = "gpt-5.4"

[model_providers.thirdparty]
name = "Thirdparty"
base_url = "https://thirdparty.example/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "stored-bearer-key"
"#;

    let provider = Provider::with_id(
        "thirdparty-1".to_string(),
        "Thirdparty".to_string(),
        json!({
            "auth": {},
            "config": stored_config,
        }),
        None,
    );

    let manager = config
        .get_manager_mut(&AppType::Codex)
        .expect("codex manager");
    manager
        .providers
        .insert("thirdparty-1".to_string(), provider);
    manager.current = "thirdparty-1".to_string();

    ConfigService::sync_current_providers_to_live(&mut config).expect("sync codex live");

    let manager = config.get_manager(&AppType::Codex).expect("codex manager");
    let synced = manager
        .providers
        .get("thirdparty-1")
        .expect("provider survives sync");

    assert_eq!(
        synced
            .settings_config
            .pointer("/auth/OPENAI_API_KEY")
            .and_then(|v| v.as_str()),
        Some("stored-bearer-key"),
        "config-only bearer token must be backfilled into stored auth.OPENAI_API_KEY"
    );

    let synced_cfg = synced
        .settings_config
        .get("config")
        .and_then(|v| v.as_str())
        .expect("config string");
    assert!(
        !synced_cfg.contains("experimental_bearer_token"),
        "live-only bearer token should not be persisted in stored provider config"
    );
}

#[test]
fn sync_codex_provider_preserves_user_model_provider_id_after_migration() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();

    let legacy_auth = json!({ "OPENAI_API_KEY": "rightcode-key" });
    let legacy_config = r#"model_provider = "rightcode"
model = "gpt-5.4"

[model_providers.rightcode]
name = "RightCode"
base_url = "https://rightcode.example/v1"
wire_api = "responses"
requires_openai_auth = true
"#;
    ogg_switch_lib::write_codex_live_atomic(&legacy_auth, Some(legacy_config))
        .expect("seed existing Codex live config");

    let mut config = MultiAppConfig::default();
    let provider_config = json!({
        "auth": {
            "OPENAI_API_KEY": "fresh-key"
        },
        "config": r#"model_provider = "aihubmix"
model = "gpt-5.4"

[model_providers.aihubmix]
name = "AiHubMix"
base_url = "https://aihubmix.example/v1"
wire_api = "responses"
requires_openai_auth = true
"#
    });

    let provider = Provider::with_id(
        "codex-1".to_string(),
        "Codex Test".to_string(),
        provider_config,
        None,
    );

    let manager = config
        .get_manager_mut(&AppType::Codex)
        .expect("codex manager");
    manager.providers.insert("codex-1".to_string(), provider);
    manager.current = "codex-1".to_string();

    ConfigService::sync_current_providers_to_live(&mut config).expect("sync codex live");

    let toml_text =
        fs::read_to_string(ogg_switch_lib::get_codex_config_path()).expect("read config.toml");
    let parsed: toml::Value = toml::from_str(&toml_text).expect("parse config.toml");

    assert_eq!(
        parsed.get("model_provider").and_then(|v| v.as_str()),
        Some("aihubmix"),
        "ConfigService sync should preserve user-editable model_provider after the one-time migration"
    );

    let model_providers = parsed
        .get("model_providers")
        .and_then(|v| v.as_table())
        .expect("model_providers should exist");
    assert!(
        model_providers.get("custom").is_none(),
        "provider sync should not force user-edited provider ids back to custom"
    );
    assert_eq!(
        model_providers
            .get("aihubmix")
            .and_then(|v| v.get("base_url"))
            .and_then(|v| v.as_str()),
        Some("https://aihubmix.example/v1")
    );

    let synced_cfg = config
        .get_manager(&AppType::Codex)
        .and_then(|manager| manager.providers.get("codex-1"))
        .and_then(|provider| provider.settings_config.get("config"))
        .and_then(|v| v.as_str())
        .expect("synced config string");
    assert!(
        synced_cfg.contains("[model_providers.aihubmix]"),
        "ConfigService should restore the provider-specific id before writing stored config"
    );
}

#[test]
fn sync_codex_provider_missing_auth_returns_error() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();

    let mut config = MultiAppConfig::default();
    let provider = Provider::with_id(
        "codex-missing-auth".to_string(),
        "No Auth".to_string(),
        json!({
            "config": "model = \"test\""
        }),
        None,
    );
    let manager = config
        .get_manager_mut(&AppType::Codex)
        .expect("codex manager");
    manager.providers.insert(provider.id.clone(), provider);
    manager.current = "codex-missing-auth".to_string();

    let err = ConfigService::sync_current_providers_to_live(&mut config)
        .expect_err("sync should fail when auth missing");
    match err {
        ogg_switch_lib::AppError::Config(msg) => {
            assert!(msg.contains("auth"), "error message should mention auth");
        }
        other => panic!("unexpected error variant: {other:?}"),
    }

    // 确认未产生任何 live 配置文件
    assert!(
        !ogg_switch_lib::get_codex_auth_path().exists(),
        "auth.json should not be created on failure"
    );
    assert!(
        !ogg_switch_lib::get_codex_config_path().exists(),
        "config.toml should not be created on failure"
    );
}

#[test]
fn write_codex_live_atomic_persists_auth_and_config() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();

    let auth = json!({ "OPENAI_API_KEY": "dev-key" });
    let config_text = r#"
[mcp_servers.echo]
type = "stdio"
command = "echo"
args = ["ok"]
"#;

    ogg_switch_lib::write_codex_live_atomic(&auth, Some(config_text))
        .expect("atomic write should succeed");

    let auth_path = ogg_switch_lib::get_codex_auth_path();
    let config_path = ogg_switch_lib::get_codex_config_path();
    assert!(auth_path.exists(), "auth.json should be created");
    assert!(config_path.exists(), "config.toml should be created");

    let stored_auth: serde_json::Value =
        ogg_switch_lib::read_json_file(&auth_path).expect("read auth");
    assert_eq!(stored_auth, auth, "auth.json should match input");

    let stored_config = std::fs::read_to_string(&config_path).expect("read config");
    assert!(
        stored_config.contains("mcp_servers.echo"),
        "config.toml should contain serialized table"
    );
}

#[test]
fn write_codex_live_atomic_rolls_back_auth_when_config_write_fails() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();

    let auth_path = ogg_switch_lib::get_codex_auth_path();
    if let Some(parent) = auth_path.parent() {
        std::fs::create_dir_all(parent).expect("create codex dir");
    }
    std::fs::write(&auth_path, r#"{"OPENAI_API_KEY":"legacy"}"#).expect("seed auth");

    let config_path = ogg_switch_lib::get_codex_config_path();
    std::fs::create_dir_all(&config_path).expect("create blocking directory");

    let auth = json!({ "OPENAI_API_KEY": "new-key" });
    let config_text = r#"[mcp_servers.sample]
type = "stdio"
command = "noop"
"#;

    let err = ogg_switch_lib::write_codex_live_atomic(&auth, Some(config_text))
        .expect_err("config write should fail when target is directory");
    match err {
        ogg_switch_lib::AppError::Io { path, .. } => {
            assert!(
                path.ends_with("config.toml"),
                "io error path should point to config.toml"
            );
        }
        ogg_switch_lib::AppError::IoContext { context, .. } => {
            assert!(
                context.contains("config.toml"),
                "error context should mention config path"
            );
        }
        other => panic!("unexpected error variant: {other:?}"),
    }

    let stored = std::fs::read_to_string(&auth_path).expect("read existing auth");
    assert!(
        stored.contains("legacy"),
        "auth.json should roll back to legacy content"
    );
    assert!(
        std::fs::metadata(&config_path)
            .expect("config path metadata")
            .is_dir(),
        "config path should remain a directory after failure"
    );
}

#[test]
fn create_backup_skips_missing_file() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let config_path = home.join(".cc-switch").join("config.json");

    // 未创建文件时应返回空字符串，不报错
    let result = ConfigService::create_backup(&config_path).expect("create backup");
    assert!(
        result.is_empty(),
        "expected empty backup id when config file missing"
    );
}

#[test]
fn create_backup_generates_snapshot_file() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let config_dir = home.join(".cc-switch");
    let config_path = config_dir.join("config.json");
    fs::create_dir_all(&config_dir).expect("prepare config dir");
    fs::write(&config_path, r#"{"version":2}"#).expect("write config file");

    let backup_id = ConfigService::create_backup(&config_path).expect("backup success");
    assert!(
        !backup_id.is_empty(),
        "backup id should contain timestamp information"
    );

    let backup_path = config_dir.join("backups").join(format!("{backup_id}.json"));
    assert!(
        backup_path.exists(),
        "expected backup file at {}",
        backup_path.display()
    );

    let backup_content = fs::read_to_string(&backup_path).expect("read backup");
    assert!(
        backup_content.contains(r#""version":2"#),
        "backup content should match original config"
    );
}

#[test]
fn create_backup_retains_only_latest_entries() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let config_dir = home.join(".cc-switch");
    let config_path = config_dir.join("config.json");
    fs::create_dir_all(&config_dir).expect("prepare config dir");
    fs::write(&config_path, r#"{"version":3}"#).expect("write config file");

    let backups_dir = config_dir.join("backups");
    fs::create_dir_all(&backups_dir).expect("create backups dir");
    for idx in 0..12 {
        let manual = backups_dir.join(format!("manual_{idx:02}.json"));
        fs::write(&manual, format!("{{\"idx\":{idx}}}")).expect("seed manual backup");
    }

    std::thread::sleep(std::time::Duration::from_secs(1));

    let latest_backup_id =
        ConfigService::create_backup(&config_path).expect("create backup with cleanup");
    assert!(
        !latest_backup_id.is_empty(),
        "backup id should not be empty when config exists"
    );

    let entries: Vec<_> = fs::read_dir(&backups_dir)
        .expect("read backups dir")
        .filter_map(|entry| entry.ok())
        .collect();
    assert!(
        entries.len() <= 10,
        "expected backups to be trimmed to at most 10 files, got {}",
        entries.len()
    );

    let latest_path = backups_dir.join(format!("{latest_backup_id}.json"));
    assert!(
        latest_path.exists(),
        "latest backup {} should be preserved",
        latest_path.display()
    );

    // 进一步确认保留的条目包含一些历史文件，说明清理逻辑仅裁剪多余部分
    let manual_kept = entries
        .iter()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .any(|name| name.starts_with("manual_"));
    assert!(
        manual_kept,
        "cleanup should keep part of the older backups to maintain history"
    );
}

#[test]
fn sync_gemini_packycode_sets_security_selected_type() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let mut config = MultiAppConfig::default();
    {
        let manager = config
            .get_manager_mut(&AppType::Gemini)
            .expect("gemini manager");
        manager.current = "packy-1".to_string();
        manager.providers.insert(
            "packy-1".to_string(),
            Provider::with_id(
                "packy-1".to_string(),
                "PackyCode".to_string(),
                json!({
                    "env": {
                        "GEMINI_API_KEY": "pk-key",
                        "GOOGLE_GEMINI_BASE_URL": "https://api-slb.packyapi.com"
                    }
                }),
                Some("https://www.packyapi.com".to_string()),
            ),
        );
    }

    ConfigService::sync_current_providers_to_live(&mut config)
        .expect("syncing gemini live should succeed");

    // security field is written to ~/.gemini/settings.json, not ~/.cc-switch/settings.json
    let gemini_settings = home.join(".gemini").join("settings.json");
    assert!(
        gemini_settings.exists(),
        "Gemini settings.json should exist at {}",
        gemini_settings.display()
    );

    let raw = std::fs::read_to_string(&gemini_settings).expect("read gemini settings.json");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("parse gemini settings.json");
    assert_eq!(
        value
            .pointer("/security/auth/selectedType")
            .and_then(|v| v.as_str()),
        Some("gemini-api-key"),
        "syncing PackyCode Gemini should enforce security.auth.selectedType in Gemini settings"
    );
}

#[test]
fn sync_gemini_google_official_sets_oauth_security() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let mut config = MultiAppConfig::default();
    {
        let manager = config
            .get_manager_mut(&AppType::Gemini)
            .expect("gemini manager");
        manager.current = "google-official".to_string();
        let mut provider = Provider::with_id(
            "google-official".to_string(),
            "Google".to_string(),
            json!({
                "env": {}
            }),
            Some("https://ai.google.dev".to_string()),
        );
        provider.meta = Some(ProviderMeta {
            partner_promotion_key: Some("google-official".to_string()),
            ..ProviderMeta::default()
        });
        manager
            .providers
            .insert("google-official".to_string(), provider);
    }

    ConfigService::sync_current_providers_to_live(&mut config)
        .expect("syncing google official gemini should succeed");

    // security field is written to ~/.gemini/settings.json, not ~/.cc-switch/settings.json
    let gemini_settings = home.join(".gemini").join("settings.json");
    assert!(
        gemini_settings.exists(),
        "Gemini settings should exist at {}",
        gemini_settings.display()
    );
    let gemini_raw = std::fs::read_to_string(&gemini_settings).expect("read gemini settings");
    let gemini_value: serde_json::Value =
        serde_json::from_str(&gemini_raw).expect("parse gemini settings json");
    assert_eq!(
        gemini_value
            .pointer("/security/auth/selectedType")
            .and_then(|v| v.as_str()),
        Some("oauth-personal"),
        "Gemini settings should record oauth-personal for Google Official"
    );
}

#[test]
fn export_sql_writes_to_target_path() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // Create test state with some data
    let mut config = MultiAppConfig::default();
    {
        let manager = config
            .get_manager_mut(&AppType::Claude)
            .expect("claude manager");
        manager.current = "test-provider".to_string();
        manager.providers.insert(
            "test-provider".to_string(),
            Provider::with_id(
                "test-provider".to_string(),
                "Test Provider".to_string(),
                json!({"env": {"ANTHROPIC_API_KEY": "test-key"}}),
                None,
            ),
        );
    }

    let state = create_test_state_with_config(&config).expect("create test state");

    // Export to SQL file
    let export_path = home.join("test-export.sql");
    state
        .db
        .export_sql(&export_path)
        .expect("export should succeed");

    // Verify file exists and contains data
    assert!(export_path.exists(), "export file should exist");
    let content = fs::read_to_string(&export_path).expect("read exported file");
    assert!(
        content.contains("INSERT INTO") && content.contains("providers"),
        "exported SQL should contain INSERT statements for providers"
    );
    assert!(
        content.contains("test-provider"),
        "exported SQL should contain test data"
    );
}

#[test]
fn export_sql_returns_error_for_invalid_path() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    // Try to export to an invalid path (nonexistent parent or invalid name on Windows)
    let invalid_parent = if cfg!(windows) {
        std::env::temp_dir().join("cc-switch-test-invalid<>dir")
    } else {
        PathBuf::from("/nonexistent/directory")
    };
    let invalid_path = invalid_parent.join("export.sql");
    let err = state
        .db
        .export_sql(&invalid_path)
        .expect_err("export to invalid path should fail");
    let invalid_prefix = invalid_parent.to_string_lossy();

    // The error can be either IoContext or Io depending on where it fails
    match err {
        AppError::IoContext { context, .. } => {
            assert!(
                context.contains("原子写入失败") || context.contains("写入失败"),
                "expected IO error message about atomic write failure, got: {context}"
            );
        }
        AppError::Io { path, .. } => {
            assert!(
                path.starts_with(invalid_prefix.as_ref()),
                "expected error for {invalid_parent:?}, got: {path:?}"
            );
        }
        other => panic!("expected IoContext or Io error, got {other:?}"),
    }
}

#[test]
fn import_sql_rejects_non_cc_switch_backup() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    let import_path = home.join("not-cc-switch.sql");
    fs::write(&import_path, "CREATE TABLE x (id INTEGER);").expect("write import sql");

    let err = state
        .db
        .import_sql(&import_path)
        .expect_err("non-cc-switch sql should be rejected");

    match err {
        AppError::Localized { key, .. } => {
            assert_eq!(key, "backup.sql.invalid_format");
        }
        other => panic!("expected Localized error, got {other:?}"),
    }
}

#[test]
fn import_sql_accepts_cc_switch_exported_backup() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // Create a database with some data and export it.
    let mut config = MultiAppConfig::default();
    {
        let manager = config
            .get_manager_mut(&AppType::Claude)
            .expect("claude manager");
        manager.current = "test-provider".to_string();
        manager.providers.insert(
            "test-provider".to_string(),
            Provider::with_id(
                "test-provider".to_string(),
                "Test Provider".to_string(),
                json!({"env": {"ANTHROPIC_API_KEY": "test-key"}}),
                None,
            ),
        );
    }

    let state = create_test_state_with_config(&config).expect("create test state");
    let export_path = home.join("cc-switch-export.sql");
    state
        .db
        .export_sql(&export_path)
        .expect("export should succeed");

    // Reset database, then import into a fresh one.
    reset_test_fs();
    let state = create_test_state().expect("create test state");
    state
        .db
        .import_sql(&export_path)
        .expect("import should succeed");

    let providers = state
        .db
        .get_all_providers(AppType::Claude.as_str())
        .expect("load providers");
    assert!(
        providers.contains_key("test-provider"),
        "imported providers should contain test-provider"
    );
}
