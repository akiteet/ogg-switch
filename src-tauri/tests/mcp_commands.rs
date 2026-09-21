use std::fs;

use serde_json::json;

use ogg_switch_lib::{
    get_claude_mcp_status, get_grok_config_path, import_default_config_test_hook,
    read_claude_mcp_config, update_settings, AppError, AppSettings, AppType, McpApps, McpServer,
    McpService, MultiAppConfig, ProviderService,
};

#[path = "support.rs"]
mod support;
use support::{create_test_state_with_config, ensure_test_home, reset_test_fs, test_mutex};

#[test]
fn import_default_config_grokbuild_seeds_official_alongside_default() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let config_path = get_grok_config_path();
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).expect("create grok config dir");
    }
    fs::write(
        &config_path,
        r#"[models]
default = "grok-4.5"

[model."grok-4.5"]
model = "grok-4.5"
base_url = "https://example.com/v1"
name = "Example"
api_key = "secret"
api_backend = "responses"
context_window = 500000
"#,
    )
    .expect("seed grok config.toml");

    let mut config = MultiAppConfig::default();
    config.ensure_app(&AppType::GrokBuild);
    let state = create_test_state_with_config(&config).expect("create test state");

    import_default_config_test_hook(&state, AppType::GrokBuild)
        .expect("import default config succeeds");

    let providers = state
        .db
        .get_all_providers(AppType::GrokBuild.as_str())
        .expect("get all providers");
    assert!(
        providers.get("default").is_some(),
        "live imported as default"
    );

    // 初次导入已有配置时应同时补出官方入口（其它应用靠首启动主播种，
    // grokbuild 种子晚于该 flag，挂在导入动作上）
    let official = providers
        .get("grokbuild-official")
        .expect("official seed ensured alongside import");
    assert_eq!(official.category.as_deref(), Some("official"));

    // 激活的仍是导入的原配置，官方入口只是备选
    let current_id = state
        .db
        .get_current_provider(AppType::GrokBuild.as_str())
        .expect("get current provider");
    assert_eq!(current_id.as_deref(), Some("default"));
}

#[test]
fn import_default_config_grokbuild_official_live_imports_official_as_current() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    // 官方登录态的 live：无自定义模型表（允许 MCP 等其它内容）。
    // 导入的正确结果 = Grok Official 成为当前供应商，而非报错。
    let config_path = get_grok_config_path();
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).expect("create grok config dir");
    }
    fs::write(&config_path, "[mcp_servers.echo]\ncommand = \"echo\"\n")
        .expect("seed official-mode grok config.toml");

    let mut config = MultiAppConfig::default();
    config.ensure_app(&AppType::GrokBuild);
    let state = create_test_state_with_config(&config).expect("create test state");

    let imported = import_default_config_test_hook(&state, AppType::GrokBuild)
        .expect("official-mode live imports as the official provider");
    assert!(imported, "official-mode import should report success");

    let providers = state
        .db
        .get_all_providers(AppType::GrokBuild.as_str())
        .expect("get all providers");
    let official = providers
        .get("grokbuild-official")
        .expect("official entry ensured by import");
    assert_eq!(official.category.as_deref(), Some("official"));
    assert!(
        providers.get("default").is_none(),
        "official-mode live must not be imported as a custom default"
    );

    let current_id = state
        .db
        .get_current_provider(AppType::GrokBuild.as_str())
        .expect("get current provider");
    assert_eq!(
        current_id.as_deref(),
        Some("grokbuild-official"),
        "official entry should become current to mirror the live state"
    );
}

#[test]
fn startup_import_grokbuild_official_live_does_not_resurrect_official() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    // 启动自动导入走 service 层（lib.rs 启动循环直接调用它）：官方态 live
    // 必须报错且不产出任何条目——全项目惯例是启动自动导入只产出 default、
    // 从不产出官方条目，否则删掉的官方条目每次重启都会复活。
    // 官方态的成功导入只挂在手动导入的命令层。
    let config_path = get_grok_config_path();
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).expect("create grok config dir");
    }
    fs::write(&config_path, "").expect("seed empty official-mode grok config.toml");

    let mut config = MultiAppConfig::default();
    config.ensure_app(&AppType::GrokBuild);
    let state = create_test_state_with_config(&config).expect("create test state");

    ProviderService::import_default_config(&state, AppType::GrokBuild)
        .expect_err("startup auto-import must not import official-mode live");

    let providers = state
        .db
        .get_all_providers(AppType::GrokBuild.as_str())
        .expect("get all providers");
    assert!(
        providers.is_empty(),
        "startup auto-import must not create any provider from official-mode live"
    );
}

#[test]
fn import_default_config_grokbuild_broken_custom_live_still_errors() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    // 有自定义痕迹但残缺（[models] 存在、缺 [model.*]）：必须报真实错误，
    // 不能被误判成官方态静默吞掉；官方入口仍由命令层前置 ensure 补出。
    let config_path = get_grok_config_path();
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).expect("create grok config dir");
    }
    fs::write(&config_path, "[models]\ndefault = \"grok-4.5\"\n")
        .expect("seed broken custom grok config.toml");

    let mut config = MultiAppConfig::default();
    config.ensure_app(&AppType::GrokBuild);
    let state = create_test_state_with_config(&config).expect("create test state");

    import_default_config_test_hook(&state, AppType::GrokBuild)
        .expect_err("broken custom config should surface a validation error");

    let providers = state
        .db
        .get_all_providers(AppType::GrokBuild.as_str())
        .expect("get all providers");
    assert!(
        providers.get("grokbuild-official").is_some(),
        "official entry still appears via the pre-import ensure"
    );
    assert!(providers.get("default").is_none(), "nothing was imported");
    let current_id = state
        .db
        .get_current_provider(AppType::GrokBuild.as_str())
        .expect("get current provider");
    assert_ne!(
        current_id.as_deref(),
        Some("grokbuild-official"),
        "failed import must not silently activate the official entry"
    );
}

#[test]
fn import_default_config_without_live_file_returns_error() {
    use support::create_test_state;

    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    let err = import_default_config_test_hook(&state, AppType::Claude)
        .expect_err("missing live file should error");
    match err {
        AppError::Localized { zh, .. } => assert!(
            zh.contains("Claude Code 配置文件不存在"),
            "unexpected error message: {zh}"
        ),
        AppError::Message(msg) => assert!(
            msg.contains("Claude Code 配置文件不存在"),
            "unexpected error message: {msg}"
        ),
        other => panic!("unexpected error variant: {other:?}"),
    }

    // 使用数据库架构，不再检查 config.json
    // 失败的导入不应该向数据库写入任何供应商
    let providers = state
        .db
        .get_all_providers(AppType::Claude.as_str())
        .expect("get all providers");
    assert!(
        providers.is_empty(),
        "failed import should not create any providers in database"
    );
}

#[test]
fn enabling_codex_mcp_skips_when_codex_dir_missing() {
    use support::create_test_state;

    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // 确认 Codex 配置目录不存在（模拟“未安装/未运行过 Codex CLI”）
    assert!(
        !home.join(".codex").exists(),
        "~/.codex should not exist in fresh test environment"
    );

    let state = create_test_state().expect("create test state");

    // 先插入一个未启用 Codex 的 MCP 服务器（避免 upsert 触发同步）
    McpService::upsert_server(
        &state,
        McpServer {
            id: "codex-server".to_string(),
            name: "Codex Server".to_string(),
            server: json!({
                "type": "stdio",
                "command": "echo"
            }),
            apps: McpApps {
                claude: false,
                codex: false,
                gemini: false,
                grokbuild: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    )
    .expect("insert server without syncing");

    // 启用 Codex：目录缺失时应跳过写入（不创建 ~/.codex/config.toml）
    McpService::toggle_app(&state, "codex-server", AppType::Codex, true)
        .expect("toggle codex should succeed even when ~/.codex is missing");

    assert!(
        !home.join(".codex").exists(),
        "~/.codex should still not exist after skipped sync"
    );
}

#[test]
fn enabling_gemini_mcp_skips_when_gemini_dir_missing() {
    use support::create_test_state;

    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // 确认 Gemini 配置目录不存在（模拟“未安装/未运行过 Gemini CLI”）
    assert!(
        !home.join(".gemini").exists(),
        "~/.gemini should not exist in fresh test environment"
    );

    let state = create_test_state().expect("create test state");

    // 先插入一个未启用 Gemini 的 MCP 服务器（避免 upsert 触发同步）
    McpService::upsert_server(
        &state,
        McpServer {
            id: "gemini-server".to_string(),
            name: "Gemini Server".to_string(),
            server: json!({
                "type": "sse",
                "url": "https://example.com/sse"
            }),
            apps: McpApps {
                claude: false,
                codex: false,
                gemini: false,
                grokbuild: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    )
    .expect("insert server without syncing");

    // 启用 Gemini：目录缺失时应跳过写入（不创建 ~/.gemini/settings.json）
    McpService::toggle_app(&state, "gemini-server", AppType::Gemini, true)
        .expect("toggle gemini should succeed even when ~/.gemini is missing");

    assert!(
        !home.join(".gemini").exists(),
        "~/.gemini should still not exist after skipped sync"
    );
}

#[test]
fn enabling_claude_mcp_skips_when_claude_config_absent() {
    use support::create_test_state;

    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // 确认 Claude 相关目录/文件都不存在（模拟“未安装/未运行过 Claude”）
    assert!(
        !home.join(".claude").exists(),
        "~/.claude should not exist in fresh test environment"
    );
    assert!(
        !home.join(".claude.json").exists(),
        "~/.claude.json should not exist in fresh test environment"
    );

    let state = create_test_state().expect("create test state");

    // 先插入一个未启用 Claude 的 MCP 服务器（避免 upsert 触发同步）
    McpService::upsert_server(
        &state,
        McpServer {
            id: "claude-server".to_string(),
            name: "Claude Server".to_string(),
            server: json!({
                "type": "stdio",
                "command": "echo"
            }),
            apps: McpApps {
                claude: false,
                codex: false,
                gemini: false,
                grokbuild: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    )
    .expect("insert server without syncing");

    // 启用 Claude：配置缺失时应跳过写入（不创建 ~/.claude.json）
    McpService::toggle_app(&state, "claude-server", AppType::Claude, true)
        .expect("toggle claude should succeed even when ~/.claude is missing");

    assert!(
        !home.join(".claude.json").exists(),
        "~/.claude.json should still not exist after skipped sync"
    );
}

#[test]
fn custom_claude_dir_read_only_mcp_queries_do_not_create_profile() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let home_mcp_path = home.join(".claude.json");
    fs::write(
        &home_mcp_path,
        serde_json::to_string_pretty(&json!({
            "mcpServers": {
                "home-only": {
                    "type": "stdio",
                    "command": "home-command"
                }
            },
            "profileSentinel": "home-profile"
        }))
        .expect("serialize default profile"),
    )
    .expect("seed default Claude profile");

    let custom_dir = home.join("profiles").join("work").join(".claude");
    fs::create_dir_all(&custom_dir).expect("create custom claude dir");
    update_settings(AppSettings {
        claude_config_dir: Some(custom_dir.to_string_lossy().to_string()),
        ..AppSettings::default()
    })
    .expect("set custom claude config dir");

    let expected_mcp_path = custom_dir.join(".claude.json");
    assert!(
        !expected_mcp_path.exists(),
        "custom profile should start without a live MCP file"
    );

    let status =
        futures::executor::block_on(get_claude_mcp_status()).expect("get Claude MCP status");
    assert_eq!(
        status.user_config_path,
        expected_mcp_path.to_string_lossy(),
        "status should report the custom profile MCP path"
    );
    assert!(
        !status.user_config_exists,
        "status should report missing custom profile MCP file"
    );
    let text =
        futures::executor::block_on(read_claude_mcp_config()).expect("read Claude MCP config");
    assert_eq!(text, None, "missing custom profile should read as None");
    assert!(
        !expected_mcp_path.exists(),
        "read-only MCP queries should not copy or create the custom profile"
    );
}
