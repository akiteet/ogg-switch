use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

use crate::config::{delete_file, get_home_dir, write_text_file};
use crate::error::AppError;
use crate::provider::Provider;

pub const DEFAULT_MODEL: &str = "grok-4.6";
pub const DEFAULT_API_BACKEND: &str = "responses";
pub const DEFAULT_CONTEXT_WINDOW: i64 = 500_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrokModelConfig {
    pub profile: String,
    pub model: String,
    pub base_url: String,
    pub name: String,
    pub api_key: Option<String>,
    pub env_key: Option<String>,
    pub api_backend: String,
    pub context_window: i64,
}

/// Grok Build configuration directory (`~/.grok`).
pub fn get_grok_config_dir() -> PathBuf {
    crate::settings::get_grok_override_dir().unwrap_or_else(|| get_home_dir().join(".grok"))
}

/// Grok Build live configuration path (`~/.grok/config.toml`).
pub fn get_grok_config_path() -> PathBuf {
    get_grok_config_dir().join("config.toml")
}

fn required_non_empty_string<'a>(
    table: &'a toml::value::Table,
    key: &str,
) -> Result<&'a str, AppError> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::localized(
                "provider.grokbuild.field.missing",
                format!("Grok Build 配置缺少有效的 {key} 字段"),
                format!("Grok Build configuration is missing a valid {key} field"),
            )
        })
}

fn optional_non_empty_string(table: &toml::value::Table, key: &str) -> Option<String> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// Syntax-only validation for a Grok Build config document (empty allowed).
///
/// 官方条目走 Grok CLI 自带的 xAI OAuth 登录，config.toml 不需要（通常也没有）
/// 自定义模型表：空文档合法，非空只要求 TOML 语法合法。live 层的读写与官方
/// 快照校验都用它；"必须有完整自定义模型表"的强校验见 `validate_config_toml`。
pub fn validate_config_toml_syntax(config_toml: &str) -> Result<(), AppError> {
    if config_toml.trim().is_empty() {
        return Ok(());
    }
    config_toml
        .parse::<toml::Value>()
        .map(|_| ())
        .map_err(|error| {
            AppError::localized(
                "provider.grokbuild.config.invalid_toml",
                format!("Grok Build config.toml 格式错误: {error}"),
                format!("Invalid Grok Build config.toml: {error}"),
            )
        })
}

/// Whether a live config document represents the official login state.
///
/// 官方态 = 语法合法且完全没有自定义模型痕迹（无 `[models]` 也无 `[model.*]`，
/// 允许 `[mcp_servers]` 等其它内容）。只要出现过任一自定义键就返回 false，
/// 让残缺的自定义配置继续走 `validate_config_toml` 报出真实错误，
/// 而不是被误判成官方态静默吞掉。语法不合法同样返回 false。
pub fn is_official_live_config(config_toml: &str) -> bool {
    let Ok(document) = config_toml.parse::<toml::Value>() else {
        return false;
    };
    document
        .as_table()
        .is_some_and(|root| !root.contains_key("models") && !root.contains_key("model"))
}

/// Validate the provider-owned Grok Build TOML document.
pub fn validate_config_toml(config_toml: &str) -> Result<(), AppError> {
    let document = config_toml.parse::<toml::Value>().map_err(|error| {
        AppError::localized(
            "provider.grokbuild.config.invalid_toml",
            format!("Grok Build config.toml 格式错误: {error}"),
            format!("Invalid Grok Build config.toml: {error}"),
        )
    })?;

    let root = document.as_table().ok_or_else(|| {
        AppError::localized(
            "provider.grokbuild.config.not_table",
            "Grok Build 配置必须是 TOML 表结构",
            "Grok Build configuration must be a TOML table",
        )
    })?;
    let models = root
        .get("models")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| {
            AppError::localized(
                "provider.grokbuild.models.missing",
                "Grok Build 配置缺少 [models]",
                "Grok Build configuration is missing [models]",
            )
        })?;
    let default_model = required_non_empty_string(models, "default")?;
    let model_entries = root
        .get("model")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| {
            AppError::localized(
                "provider.grokbuild.model.missing",
                "Grok Build 配置缺少 [model.<name>]",
                "Grok Build configuration is missing [model.<name>]",
            )
        })?;
    let selected_model = model_entries
        .get(default_model)
        .and_then(toml::Value::as_table)
        .ok_or_else(|| {
            AppError::localized(
                "provider.grokbuild.default_model.missing",
                format!("Grok Build 配置缺少 [model.\"{default_model}\"]"),
                format!("Grok Build configuration is missing [model.\"{default_model}\"]"),
            )
        })?;

    required_non_empty_string(selected_model, "model")?;
    required_non_empty_string(selected_model, "base_url")?;
    // `name` is optional: it is only a display label for the grok model picker.
    // When absent, the model id is used as the display name (see takeover and
    // extract_model_config fallbacks), so a missing `name` must not fail
    // validation — a required check here made every name-less config fail with
    // "Missing name" and blocked takeover/switch midway.
    if optional_non_empty_string(selected_model, "api_key").is_none()
        && optional_non_empty_string(selected_model, "env_key").is_none()
    {
        return Err(AppError::localized(
            "provider.grokbuild.credentials.missing",
            "Grok Build 配置缺少有效的 api_key 或 env_key 字段",
            "Grok Build configuration is missing a valid api_key or env_key field",
        ));
    }
    required_non_empty_string(selected_model, "api_backend")?;

    selected_model
        .get("context_window")
        .and_then(toml::Value::as_integer)
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            AppError::localized(
                "provider.grokbuild.context_window.invalid",
                "Grok Build context_window 必须是正整数",
                "Grok Build context_window must be a positive integer",
            )
        })?;

    Ok(())
}

pub fn extract_model_config(config_toml: &str) -> Option<GrokModelConfig> {
    let document = config_toml.parse::<toml::Value>().ok()?;
    let root = document.as_table()?;
    let default_model = root
        .get("models")?
        .as_table()?
        .get("default")?
        .as_str()?
        .trim();
    let selected_model = root
        .get("model")?
        .as_table()?
        .get(default_model)?
        .as_table()?;
    Some(GrokModelConfig {
        profile: default_model.to_string(),
        model: selected_model.get("model")?.as_str()?.trim().to_string(),
        base_url: selected_model
            .get("base_url")?
            .as_str()?
            .trim_end_matches('/')
            .to_string(),
        name: selected_model
            .get("name")
            .and_then(toml::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(default_model)
            .to_string(),
        api_key: optional_non_empty_string(selected_model, "api_key"),
        env_key: optional_non_empty_string(selected_model, "env_key"),
        api_backend: selected_model
            .get("api_backend")?
            .as_str()?
            .trim()
            .to_string(),
        context_window: selected_model.get("context_window")?.as_integer()?,
    })
}

pub fn extract_credentials(config_toml: &str) -> Option<(String, String)> {
    let config = extract_model_config(config_toml)?;
    // Credentials only come from two explicit, config-declared sources:
    //   1. an inline `api_key`, or
    //   2. the process env var named by `env_key`.
    //
    // Deliberately NO unconditional fallback to `XAI_API_KEY`: silently
    // substituting a different account's key (when the declared `env_key` var is
    // unset) would leak that key to whatever `base_url` this config points at.
    // An unset/missing declared credential must surface as "no credential"
    // (None) so callers can fail loudly rather than transmit the wrong secret.
    let api_key = config.api_key.or_else(|| {
        config
            .env_key
            .as_deref()
            .and_then(|key| std::env::var(key).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })?;
    Some((config.base_url, api_key))
}

pub fn extract_inline_api_key(config_toml: &str) -> Option<String> {
    extract_model_config(config_toml)?.api_key
}

pub fn extract_base_url(config_toml: &str) -> Option<String> {
    Some(extract_model_config(config_toml)?.base_url)
}

fn update_selected_model_string(
    config_toml: &str,
    field: &str,
    value: &str,
) -> Result<String, AppError> {
    let mut document = config_toml
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| {
            AppError::localized(
                "provider.grokbuild.config.invalid_toml",
                format!("Grok Build config.toml 格式错误: {error}"),
                format!("Invalid Grok Build config.toml: {error}"),
            )
        })?;
    let default_model = document
        .get("models")
        .and_then(|item| item.get("default"))
        .and_then(toml_edit::Item::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .ok_or_else(|| {
            AppError::localized(
                "provider.grokbuild.default_model.missing",
                "Grok Build 配置缺少 models.default",
                "Grok Build configuration is missing models.default",
            )
        })?
        .to_string();

    let selected_model = document
        .get_mut("model")
        .and_then(|item| item.get_mut(&default_model))
        .and_then(toml_edit::Item::as_table_like_mut)
        .ok_or_else(|| {
            AppError::localized(
                "provider.grokbuild.default_model.missing",
                format!("Grok Build 配置缺少 [model.\"{default_model}\"]"),
                format!("Grok Build configuration is missing [model.\"{default_model}\"]"),
            )
        })?;
    selected_model.insert(field, toml_edit::value(value));
    Ok(document.to_string())
}

pub fn apply_proxy_takeover(
    config_toml: &str,
    proxy_base_url: &str,
    token_placeholder: &str,
) -> Result<String, AppError> {
    let mut document = config_toml
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| {
            AppError::localized(
                "provider.grokbuild.config.invalid_toml",
                format!("Grok Build config.toml 格式错误: {error}"),
                format!("Invalid Grok Build config.toml: {error}"),
            )
        })?;
    let model_entries = document
        .get_mut("model")
        .and_then(toml_edit::Item::as_table_like_mut)
        .ok_or_else(|| {
            AppError::localized(
                "provider.grokbuild.model.missing",
                "Grok Build 配置缺少 [model.<name>]",
                "Grok Build configuration is missing [model.<name>]",
            )
        })?;

    // grok CLI can switch among every configured model at runtime, so the
    // takeover must route ALL entries through the local proxy (which only
    // serves the Responses protocol at /grokbuild/v1/responses). Updating only
    // the selected entry left the others pointing straight at the upstream
    // relay, where grok CLI 1.0.30 fails (missing `sequence_number`, empty
    // tool names). `name` is auto-filled with the entry key so the model
    // picker shows the model id and `name`-required validation keeps passing.
    let entry_keys: Vec<String> = model_entries
        .iter()
        .map(|(key, _)| key.to_string())
        .collect();
    if entry_keys.is_empty() {
        return Err(AppError::localized(
            "provider.grokbuild.model.missing",
            "Grok Build 配置缺少 [model.<name>]",
            "Grok Build configuration is missing [model.<name>]",
        ));
    }
    for key in &entry_keys {
        let Some(entry) = model_entries
            .get_mut(key.as_str())
            .and_then(toml_edit::Item::as_table_like_mut)
        else {
            continue;
        };
        entry.insert("base_url", toml_edit::value(proxy_base_url));
        entry.insert("api_key", toml_edit::value(token_placeholder));
        entry.insert("api_backend", toml_edit::value(DEFAULT_API_BACKEND));
        let has_name = entry
            .get("name")
            .and_then(toml_edit::Item::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some();
        if !has_name {
            entry.insert("name", toml_edit::value(key.as_str()));
        }
    }

    // [endpoints] 里的 models_base_url 一物两用,缺一不可:
    //   1. 决定 grok CLI 从哪里拉取模型目录(/model 选择器的数据源);
    //   2. 决定认证方式 —— 设置了它,CLI 使用 API Key 认证(Authorization: Bearer);
    //      没设置,CLI 回落到官方 session/OIDC 登录态(session auth)。
    // 只改 [model.*] 而不写这一段,CLI 会停留在官方认证模式并去官方拉目录,
    // 结果就是"切到中转站仍走官方认证、/model 切换无效"。
    // 保留 [endpoints] 下原有的其它键,只覆盖这两个 base_url。
    let endpoints = document
        .entry("endpoints")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    if let Some(table) = endpoints.as_table_like_mut() {
        table.insert("models_base_url", toml_edit::value(proxy_base_url));
        table.insert("xai_api_base_url", toml_edit::value(proxy_base_url));
    }
    Ok(document.to_string())
}

/// 移除 [endpoints] 里的 models_base_url / xai_api_base_url(切回官方登录态时用),
/// 保留该段下其它键;若清理后该段为空则整段删除。
pub fn strip_proxy_endpoints(config_toml: &str) -> Result<String, AppError> {
    let mut document = config_toml
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| {
            AppError::localized(
                "provider.grokbuild.config.invalid_toml",
                format!("Grok Build config.toml 格式错误: {error}"),
                format!("Invalid Grok Build config.toml: {error}"),
            )
        })?;
    let should_remove = match document
        .get_mut("endpoints")
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        Some(table) => {
            table.remove("models_base_url");
            table.remove("xai_api_base_url");
            table.is_empty()
        }
        None => false,
    };
    if should_remove {
        document.remove("endpoints");
    }
    Ok(document.to_string())
}

pub fn update_api_key(config_toml: &str, api_key: &str) -> Result<String, AppError> {
    update_selected_model_string(config_toml, "api_key", api_key)
}

pub fn has_proxy_placeholder(config_toml: &str, token_placeholder: &str) -> bool {
    extract_model_config(config_toml)
        .and_then(|config| config.api_key)
        .is_some_and(|api_key| api_key == token_placeholder)
}

pub fn base_url_matches(config_toml: &str, predicate: impl FnOnce(&str) -> bool) -> bool {
    extract_model_config(config_toml).is_some_and(|config| predicate(&config.base_url))
}

/// Remove MCP projections from a provider-owned Grok Build settings snapshot.
/// MCP servers are owned by the database and projected into live config.toml.
pub fn strip_grok_mcp_servers_from_settings(settings: &mut Value) -> Result<(), AppError> {
    let Some(config_text) = settings
        .get("config")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(());
    };
    if !config_text.contains("mcp") {
        return Ok(());
    }

    let mut document = config_text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| AppError::Message(format!("Invalid Grok Build config.toml: {error}")))?;
    let mut changed = document.as_table_mut().remove("mcp_servers").is_some();
    if let Some(mcp_table) = document
        .get_mut("mcp")
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        if mcp_table.remove("servers").is_some() {
            changed = true;
        }
        if mcp_table.is_empty() {
            document.as_table_mut().remove("mcp");
        }
    }

    if changed {
        if let Some(object) = settings.as_object_mut() {
            object.insert("config".to_string(), Value::String(document.to_string()));
        }
    }
    Ok(())
}

/// 一个模型条目的完整信息,用于构造 grok CLI 的 `/models` 响应。
#[derive(Debug, Clone, serde::Serialize)]
pub struct GrokModelEntryInfo {
    /// `[model."<id>"]` 的键名。
    pub id: String,
    /// 真正发给上游的模型名(条目的 `model` 字段,缺省回落为键名)。
    pub model: String,
    /// 选择器里显示的标签(条目的 `name` 字段,缺省回落为键名)。
    pub name: String,
    /// 上下文窗口,CLI 的自动压缩依据。
    pub context_window: i64,
    /// 线协议(responses / chat_completions)。
    pub api_backend: String,
}

/// 读取 live 配置中全部 `[model.*]` 条目的完整信息。
///
/// grok CLI 的 `/models` 解析器要求每个条目至少带 `model` 或 `context_window`,
/// 否则会整条丢弃(`Skipping model at index N: missing required field ...`),
/// 最终报 "model catalog fetch returned no models"。只回 `id` 是不够的。
pub fn list_grok_model_entries(config_toml: &str) -> Vec<GrokModelEntryInfo> {
    let Ok(document) = config_toml.parse::<toml::Value>() else {
        return Vec::new();
    };
    let Some(table) = document.get("model").and_then(toml::Value::as_table) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for (key, value) in table {
        let id = key.trim();
        if id.is_empty() {
            continue;
        }
        let str_at = |field: &str| {
            value
                .get(field)
                .and_then(toml::Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(ToString::to_string)
        };
        entries.push(GrokModelEntryInfo {
            id: id.to_string(),
            model: str_at("model").unwrap_or_else(|| id.to_string()),
            name: str_at("name").unwrap_or_else(|| id.to_string()),
            context_window: value
                .get("context_window")
                .and_then(toml::Value::as_integer)
                .filter(|size| *size > 0)
                .unwrap_or(DEFAULT_CONTEXT_WINDOW),
            api_backend: str_at("api_backend").unwrap_or_else(|| DEFAULT_API_BACKEND.to_string()),
        });
    }
    entries
}

/// 建立 `[model."<条目名>"]` → 该条目 `model` 字段(真正发给上游的模型名)的映射。
///
/// grok CLI 的 /model 切换的是条目名,代理必须把它翻译成上游模型名。
/// 缺少这层映射时代理会退化成"永远使用默认条目",使 /model 变成假切换。
pub fn grok_model_entry_upstream_map(
    config_toml: &str,
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(document) = config_toml.parse::<toml::Value>() else {
        return map;
    };
    let Some(table) = document.get("model").and_then(toml::Value::as_table) else {
        return map;
    };
    for (key, value) in table {
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let upstream = value
            .get("model")
            .and_then(toml::Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .unwrap_or(key);
        map.insert(key.to_string(), upstream.to_string());
    }
    map
}

/// Read the live `~/.grok/config.toml` as a provider settings snapshot.
///
/// 只做 TOML 语法校验：live 处于官方态（无自定义模型表）时同样需要能被
/// 读取，供切换回填与界面展示使用。需要"完整自定义模型配置"的导入路径
/// 由调用方自行叠加 `validate_config_toml`。
pub fn read_grok_live_settings() -> Result<Value, AppError> {
    let path = get_grok_config_path();
    if !path.exists() {
        return Err(AppError::localized(
            "grokbuild.config.missing",
            "Grok Build 配置文件不存在",
            "Grok Build configuration file not found",
        ));
    }

    let config = fs::read_to_string(&path).map_err(|error| AppError::io(&path, error))?;
    validate_config_toml_syntax(&config)?;
    Ok(json!({ "config": config }))
}

pub fn write_grok_provider_live(provider: &Provider) -> Result<(), AppError> {
    let settings = provider.settings_config.as_object().ok_or_else(|| {
        AppError::localized(
            "provider.grokbuild.settings.not_object",
            "Grok Build 配置必须是 JSON 对象",
            "Grok Build configuration must be a JSON object",
        )
    })?;
    let config = settings
        .get("config")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::localized(
                "provider.grokbuild.config.missing",
                "Grok Build 配置缺少 config 字段",
                "Grok Build configuration is missing the config field",
            )
        })?;

    // 官方条目不注入自定义模型表：按快照原样写回（首次为空文件），
    // Grok CLI 回落到官方内置模型 + 自带 OAuth 登录；MCP 投影随后由
    // 切换流程重新补写。非官方供应商必须携带完整的自定义模型配置。
    let is_official = provider.category.as_deref() == Some("official");
    if !is_official {
        validate_config_toml(config)?;
    }

    write_grok_live_settings(&json!({ "config": config }))?;

    // 切到非官方供应商后必须清掉 auth.json 里的官方 OIDC 会话(issue #6589):
    // 否则 grok CLI 会把官方 token 发往第三方 base_url,持续 401 Invalid token,
    // 触发 401 → 自动 OIDC 登录 → 再 401 的死循环。
    // 顺序要求:先成功写入 config.toml(上面已 return on error),再删 auth.json;
    // 删除失败只告警,不阻断切换。
    //
    // 但「删掉 = 登录态丢失」曾让用户每次切回官方都要重新 `grok login`，所以删除前
    // 先把文件备份到 OGG 的备份目录，切回官方时（下面的 else 分支）自动恢复。
    let auth_path = get_grok_config_dir().join("auth.json");
    if !is_official {
        crate::services::cli_auth_backup::backup_cli_auth_logging("grokbuild", &auth_path);
        if let Err(error) = delete_file(&auth_path) {
            log::warn!(
                "清理官方登录态失败(不阻断切换) {}: {error}",
                auth_path.display()
            );
        }
    } else {
        // 切回官方：本机 auth.json 缺失时用备份恢复（本机现有文件永远优先——
        // 用户可能刚重新登录过）。恢复失败只告警，不阻断切换。
        crate::services::cli_auth_backup::restore_cli_auth_logging("grokbuild", &auth_path);
    }

    Ok(())
}

/// Raw live-file writer, mirroring `read_grok_live_settings` (syntax-only).
///
/// 代理接管的备份/恢复也走这里：官方态 live（无自定义模型表）必须可以
/// 原样写回。完整形状校验由 `write_grok_provider_live` 的非官方分支负责。
pub fn write_grok_live_settings(settings: &Value) -> Result<(), AppError> {
    let config = settings
        .get("config")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::localized(
                "provider.grokbuild.config.missing",
                "Grok Build 配置缺少 config 字段",
                "Grok Build configuration is missing the config field",
            )
        })?;
    validate_config_toml_syntax(config)?;
    write_text_file(&get_grok_config_path(), config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    fn valid_config() -> &'static str {
        r#"[models]
default = "grok-4.5"

[model."grok-4.5"]
model = "grok-4.5"
base_url = "https://example.com/v1"
name = "Example"
api_key = "secret"
api_backend = "responses"
context_window = 500000
"#
    }

    fn valid_env_key_config() -> &'static str {
        r#"[models]
default = "grok-env"

[model."grok-env"]
model = "grok-4.5"
base_url = "https://example.com/v1"
name = "Example Env"
env_key = "GROK_TEST_API_KEY"
api_backend = "responses"
context_window = 500000
"#
    }

    #[test]
    fn validates_expected_config_shape() {
        validate_config_toml(valid_config()).expect("valid Grok Build config");
        validate_config_toml(valid_env_key_config()).expect("valid env_key configuration");
    }

    #[test]
    fn syntax_validation_accepts_official_snapshots() {
        validate_config_toml_syntax("").expect("empty official snapshot");
        validate_config_toml_syntax("[mcp_servers.echo]\ncommand = \"echo\"\n")
            .expect("official-mode config without model tables");
        assert!(validate_config_toml_syntax("not = [valid").is_err());
    }

    #[test]
    fn official_live_config_detection() {
        // 官方态：完全没有自定义模型痕迹
        assert!(is_official_live_config(""));
        assert!(is_official_live_config("  \n# comment only\n"));
        assert!(is_official_live_config(
            "[mcp_servers.echo]\ncommand = \"echo\"\n"
        ));

        // 出现过任一自定义键（哪怕残缺）都不是官方态，交给强校验报错
        assert!(!is_official_live_config(valid_config()));
        assert!(!is_official_live_config("[models]\ndefault = \"x\"\n"));
        assert!(!is_official_live_config("[model.x]\nmodel = \"x\"\n"));

        // 语法不合法不是官方态
        assert!(!is_official_live_config("not = [valid"));
    }

    #[test]
    fn rejects_missing_selected_model_table() {
        let error = validate_config_toml("[models]\ndefault = \"grok-4.5\"\n")
            .expect_err("missing model table should fail");
        assert!(error.to_string().contains("model"));
    }

    #[test]
    fn rejects_config_without_api_key_or_env_key() {
        let config = valid_config().replace("api_key = \"secret\"\n", "");
        let error = validate_config_toml(&config).expect_err("credentials should be required");
        assert!(error.to_string().contains("api_key"));
        assert!(error.to_string().contains("env_key"));
    }

    #[test]
    fn extracts_selected_model_and_updates_takeover_fields() {
        let selected = extract_model_config(valid_config()).expect("selected model");
        assert_eq!(selected.profile, "grok-4.5");
        assert_eq!(selected.model, "grok-4.5");
        assert_eq!(selected.base_url, "https://example.com/v1");

        let updated = apply_proxy_takeover(
            valid_config(),
            "http://127.0.0.1:15721/grokbuild/v1",
            "PROXY_MANAGED",
        )
        .expect("takeover config");
        let selected = extract_model_config(&updated).expect("updated selected model");
        assert_eq!(selected.base_url, "http://127.0.0.1:15721/grokbuild/v1");
        assert_eq!(selected.api_key.as_deref(), Some("PROXY_MANAGED"));
        assert!(has_proxy_placeholder(&updated, "PROXY_MANAGED"));
    }

    #[test]
    fn takeover_preserves_env_key_profile_and_injects_inline_placeholder() {
        let direct_config = valid_env_key_config().replace(
            "api_backend = \"responses\"",
            "api_backend = \"chat_completions\"",
        );
        let updated = apply_proxy_takeover(
            &direct_config,
            "http://127.0.0.1:15721/grokbuild/v1",
            "PROXY_MANAGED",
        )
        .expect("takeover config");
        let selected = extract_model_config(&updated).expect("updated selected model");

        assert_eq!(selected.profile, "grok-env");
        assert_eq!(selected.env_key.as_deref(), Some("GROK_TEST_API_KEY"));
        assert_eq!(selected.api_key.as_deref(), Some("PROXY_MANAGED"));
        assert_eq!(selected.api_backend, DEFAULT_API_BACKEND);
    }

    #[test]
    #[serial]
    fn resolves_api_key_from_configured_environment_variable() {
        let original = std::env::var_os("GROK_TEST_API_KEY");
        std::env::set_var("GROK_TEST_API_KEY", "env-secret");

        let credentials = extract_credentials(valid_env_key_config()).expect("credentials");

        assert_eq!(credentials.0, "https://example.com/v1");
        assert_eq!(credentials.1, "env-secret");
        match original {
            Some(value) => std::env::set_var("GROK_TEST_API_KEY", value),
            None => std::env::remove_var("GROK_TEST_API_KEY"),
        }
    }

    /// 构造一个 `env_key` 指向未设置环境变量的 config——这是"声明了间接引用但
    /// 该变量不存在"的场景，修复前会静默兜底到 `XAI_API_KEY`。
    fn env_key_unset_config() -> &'static str {
        r#"[models]
default = "grok-env"

[model."grok-env"]
model = "grok-4.5"
base_url = "https://attacker.example/v1"
name = "Attacker Env"
env_key = "GROK_TEST_DEFINITELY_UNSET_VAR"
api_backend = "responses"
context_window = 500000
"#
    }

    #[test]
    #[serial]
    fn does_not_fall_back_to_xai_api_key_when_declared_env_key_is_unset() {
        // 即使进程里恰好设了 XAI_API_KEY，也不能被静默借用到别的 base_url 上。
        let original_xai = std::env::var_os("XAI_API_KEY");
        let original_unset = std::env::var_os("GROK_TEST_DEFINITELY_UNSET_VAR");
        std::env::set_var("XAI_API_KEY", "xai-secret-should-not-leak");
        std::env::remove_var("GROK_TEST_DEFINITELY_UNSET_VAR");

        let credentials = extract_credentials(env_key_unset_config());

        assert!(
            credentials.is_none(),
            "declared env_key unset must yield None, never a borrowed XAI_API_KEY; got {credentials:?}"
        );

        match original_xai {
            Some(value) => std::env::set_var("XAI_API_KEY", value),
            None => std::env::remove_var("XAI_API_KEY"),
        }
        match original_unset {
            Some(value) => std::env::set_var("GROK_TEST_DEFINITELY_UNSET_VAR", value),
            None => std::env::remove_var("GROK_TEST_DEFINITELY_UNSET_VAR"),
        }
    }

    #[test]
    fn strips_projected_mcp_servers_without_touching_model_config() {
        let mut settings = json!({
            "config": format!(
                "{}\n[mcp_servers.echo]\ncommand = \"echo\"\n",
                valid_config()
            )
        });

        strip_grok_mcp_servers_from_settings(&mut settings).expect("strip MCP servers");

        let config = settings.get("config").and_then(Value::as_str).unwrap();
        assert!(!config.contains("mcp_servers"));
        assert!(config.contains("model = \"grok-4.5\""));
        validate_config_toml(config).expect("stripped config remains valid");
    }

    #[test]
    #[serial]
    fn official_provider_roundtrips_without_custom_model_tables() {
        let temp = TempDir::new().expect("temp dir");
        let original_test_home = std::env::var_os("CC_SWITCH_TEST_HOME");
        std::env::set_var("CC_SWITCH_TEST_HOME", temp.path());

        // 官方条目：空 config 可写（清掉自定义模型表，交还 Grok CLI 官方登录）
        let mut official = Provider::with_id(
            "grokbuild-official".to_string(),
            "Grok Official".to_string(),
            json!({ "config": "" }),
            None,
        );
        official.category = Some("official".to_string());
        write_grok_provider_live(&official).expect("official empty config is writable");
        assert_eq!(
            fs::read_to_string(get_grok_config_path()).expect("read config"),
            ""
        );

        // 官方态 live（如 MCP 投影补写后）无自定义模型表，读取与原样写回都必须可用
        let official_live = "[mcp_servers.echo]\ncommand = \"echo\"\n";
        write_grok_live_settings(&json!({ "config": official_live }))
            .expect("official-mode live is writable for backup restore");
        let settings = read_grok_live_settings().expect("official-mode live is readable");
        assert_eq!(
            settings.get("config").and_then(Value::as_str),
            Some(official_live)
        );

        // 非官方供应商仍要求完整的自定义模型配置
        let custom = Provider::with_id(
            "custom".to_string(),
            "Custom".to_string(),
            json!({ "config": "" }),
            None,
        );
        assert!(write_grok_provider_live(&custom).is_err());

        match original_test_home {
            Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
            None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
        }
    }

    #[test]
    #[serial]
    fn writes_and_reads_live_config() {
        let temp = TempDir::new().expect("temp dir");
        let original_test_home = std::env::var_os("CC_SWITCH_TEST_HOME");
        std::env::set_var("CC_SWITCH_TEST_HOME", temp.path());

        let provider = Provider::with_id(
            "grok".to_string(),
            "Example".to_string(),
            json!({ "config": valid_config() }),
            None,
        );
        write_grok_provider_live(&provider).expect("write live config");

        let path = get_grok_config_path();
        assert_eq!(path, temp.path().join(".grok").join("config.toml"));
        assert_eq!(
            fs::read_to_string(path).expect("read config"),
            valid_config()
        );
        assert_eq!(
            read_grok_live_settings()
                .expect("read live settings")
                .get("config")
                .and_then(Value::as_str),
            Some(valid_config())
        );

        match original_test_home {
            Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
            None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
        }
    }

    /// 切第三方会删 auth.json（防官方 token 被发往第三方端点），但删除前必须备份，
    /// 切回官方时自动恢复——否则用户每次都要重新 `grok login`
    /// （2026-09-24 报障「每次从第三方切回官方供应商都要重新登录」）。
    #[test]
    #[serial]
    fn third_party_switch_backs_up_auth_and_official_switch_restores_it() {
        let temp = TempDir::new().expect("temp dir");
        let original_test_home = std::env::var_os("CC_SWITCH_TEST_HOME");
        std::env::set_var("CC_SWITCH_TEST_HOME", temp.path());

        let auth_path = get_grok_config_dir().join("auth.json");
        fs::create_dir_all(auth_path.parent().expect("auth parent")).expect("create grok dir");
        let session = r#"{"oidc":{"key":"official-session"}}"#;
        fs::write(&auth_path, session).expect("seed official session");

        let third_party = Provider::with_id(
            "relay".to_string(),
            "Relay".to_string(),
            json!({ "config": valid_config() }),
            None,
        );
        write_grok_provider_live(&third_party).expect("third-party switch");
        assert!(
            !auth_path.exists(),
            "third-party switch must still clear the official OIDC session"
        );
        assert!(
            crate::services::cli_auth_backup::backup_path("grokbuild").is_file(),
            "the cleared session must land in the backup before deletion"
        );

        let mut official = Provider::with_id(
            "grokbuild-official".to_string(),
            "Grok Official".to_string(),
            json!({ "config": "" }),
            None,
        );
        official.category = Some("official".to_string());
        write_grok_provider_live(&official).expect("official switch");

        assert_eq!(
            fs::read_to_string(&auth_path).expect("restored auth.json"),
            session,
            "switching back to the official provider must restore the login"
        );

        // 本机重新登录过（文件更新）时，恢复不得覆盖新登录态
        let refreshed = r#"{"oidc":{"key":"fresh-session"}}"#;
        fs::write(&auth_path, refreshed).expect("re-login");
        write_grok_provider_live(&third_party).expect("third-party switch again");
        fs::write(&auth_path, refreshed).expect("re-login during third-party");
        write_grok_provider_live(&official).expect("official switch again");
        assert_eq!(
            fs::read_to_string(&auth_path).expect("auth.json kept"),
            refreshed
        );

        match original_test_home {
            Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
            None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
        }
    }
}
