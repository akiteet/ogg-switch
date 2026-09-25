//! 官方订阅额度查询服务
//!
//! 读取 CLI 工具的已有 OAuth 凭据，查询官方订阅额度。
//! 第一层：仅读取凭据，不实现登录/刷新。

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use std::collections::{HashMap, HashSet};

use crate::config;

// ── 数据类型 ──────────────────────────────────────────────

/// 凭据状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStatus {
    Valid,
    Expired,
    NotFound,
    ParseError,
}

/// 单个限速窗口（如 5小时会话、7天周期）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaTier {
    /// 窗口标识：five_hour, seven_day, seven_day_fable, seven_day_opus 等
    pub name: String,
    /// 使用百分比 0–100
    pub utilization: f64,
    /// ISO 8601 重置时间
    pub resets_at: Option<String>,
    /// ZenMux: 已用额度（USD）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_value_usd: Option<f64>,
    /// ZenMux: 窗口上限（USD）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value_usd: Option<f64>,
}

/// 超额使用信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraUsage {
    pub is_enabled: bool,
    pub monthly_limit: Option<f64>,
    pub used_credits: Option<f64>,
    pub utilization: Option<f64>,
    pub currency: Option<String>,
}

/// 订阅额度查询结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionQuota {
    pub tool: String,
    pub credential_status: CredentialStatus,
    pub credential_message: Option<String>,
    pub success: bool,
    pub tiers: Vec<QuotaTier>,
    pub extra_usage: Option<ExtraUsage>,
    pub error: Option<String>,
    pub queried_at: Option<i64>,
}

impl SubscriptionQuota {
    pub(crate) fn not_found(tool: &str) -> Self {
        Self {
            tool: tool.to_string(),
            credential_status: CredentialStatus::NotFound,
            credential_message: None,
            success: false,
            tiers: vec![],
            extra_usage: None,
            error: None,
            queried_at: None,
        }
    }

    pub(crate) fn error(tool: &str, status: CredentialStatus, message: String) -> Self {
        Self {
            tool: tool.to_string(),
            credential_status: status,
            credential_message: Some(message.clone()),
            success: false,
            tiers: vec![],
            extra_usage: None,
            error: Some(message),
            queried_at: Some(now_millis()),
        }
    }
}

// ── Claude 凭据读取 ──────────────────────────────────────

/// Claude OAuth 凭据文件中的嵌套结构
#[derive(Deserialize)]
struct ClaudeOAuthEntry {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<serde_json::Value>,
}

/// 读取 Claude OAuth 凭据
///
/// 按优先级尝试以下来源：
/// 1. macOS Keychain (service: "Claude Code-credentials")
/// 2. 凭据文件 ~/.claude/.credentials.json
///
/// JSON 格式（两种 key 都兼容）：
/// {"claudeAiOauth": {"accessToken": "...", "expiresAt": ...}}
/// {"claude.ai_oauth": {"accessToken": "...", "expiresAt": ...}}
fn read_claude_credentials() -> (Option<String>, CredentialStatus, Option<String>) {
    // 来源 1: macOS Keychain
    #[cfg(target_os = "macos")]
    {
        if let Some(result) = read_claude_credentials_from_keychain() {
            return result;
        }
    }

    // 来源 2: 凭据文件
    read_claude_credentials_from_file()
}

/// 从 macOS Keychain 读取 Claude 凭据
#[cfg(target_os = "macos")]
fn read_claude_credentials_from_keychain(
) -> Option<(Option<String>, CredentialStatus, Option<String>)> {
    let output = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None; // Keychain 中无此条目，回退到文件
    }

    let json_str = String::from_utf8(output.stdout).ok()?;
    let json_str = json_str.trim();
    if json_str.is_empty() {
        return None;
    }

    Some(parse_claude_credentials_json(json_str))
}

/// 从文件读取 Claude 凭据
fn read_claude_credentials_from_file() -> (Option<String>, CredentialStatus, Option<String>) {
    let cred_path = config::get_claude_config_dir().join(".credentials.json");

    if !cred_path.exists() {
        return (None, CredentialStatus::NotFound, None);
    }

    let content = match std::fs::read_to_string(&cred_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to read credentials file: {e}")),
            );
        }
    };

    parse_claude_credentials_json(&content)
}

/// 解析 Claude 凭据 JSON（Keychain 和文件共用）
fn parse_claude_credentials_json(
    content: &str,
) -> (Option<String>, CredentialStatus, Option<String>) {
    let parsed: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(e) => {
            return (
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse credentials JSON: {e}")),
            );
        }
    };

    // 兼容两种 key 名
    let entry_value = parsed
        .get("claudeAiOauth")
        .or_else(|| parsed.get("claude.ai_oauth"));

    let entry_value = match entry_value {
        Some(v) => v,
        None => {
            return (
                None,
                CredentialStatus::ParseError,
                Some("No OAuth entry found in credentials".to_string()),
            );
        }
    };

    let entry: ClaudeOAuthEntry = match serde_json::from_value(entry_value.clone()) {
        Ok(e) => e,
        Err(e) => {
            return (
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse OAuth entry: {e}")),
            );
        }
    };

    let access_token = match entry.access_token {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                None,
                CredentialStatus::ParseError,
                Some("accessToken is empty or missing".to_string()),
            );
        }
    };

    // 检查 token 是否过期
    if let Some(expires_at) = entry.expires_at {
        if is_token_expired(&expires_at) {
            return (
                Some(access_token),
                CredentialStatus::Expired,
                Some("OAuth token has expired".to_string()),
            );
        }
    }

    (Some(access_token), CredentialStatus::Valid, None)
}

/// 判断 token 是否过期，兼容 Unix 时间戳（秒/毫秒）和 ISO 字符串
fn is_token_expired(expires_at: &serde_json::Value) -> bool {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    match expires_at {
        serde_json::Value::Number(n) => {
            if let Some(ts) = n.as_u64() {
                // 区分秒和毫秒（毫秒级时间戳大于 1e12）
                let ts_secs = if ts > 1_000_000_000_000 {
                    ts / 1000
                } else {
                    ts
                };
                ts_secs < now_secs
            } else {
                false
            }
        }
        serde_json::Value::String(s) => {
            // 尝试解析 ISO 8601 格式
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                (dt.timestamp() as u64) < now_secs
            } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
            {
                (dt.and_utc().timestamp() as u64) < now_secs
            } else {
                false // 无法解析时不视为过期
            }
        }
        _ => false,
    }
}

// ── Claude API 查询 ──────────────────────────────────────

/// Claude OAuth 用量 API 响应中的单个窗口
#[derive(Deserialize)]
struct ApiUsageWindow {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

/// `limits[]` 中的窗口使用 `percent`，而非旧顶层窗口的 `utilization`。
#[derive(Deserialize)]
struct ApiScopedUsageWindow {
    percent: f64,
    resets_at: Option<String>,
}

/// Claude OAuth 用量 API 响应中的超额用量
#[derive(Deserialize)]
struct ApiExtraUsage {
    is_enabled: Option<bool>,
    monthly_limit: Option<f64>,
    used_credits: Option<f64>,
    utilization: Option<f64>,
    currency: Option<String>,
}

/// 已知的 Claude 用量窗口名称；未知的旧格式窗口仍保留原名称。
pub const TIER_FIVE_HOUR: &str = "five_hour";
pub const TIER_SEVEN_DAY: &str = "seven_day";
/// 内部统一名称：Fable 实际由 `limits[].scope.model` 标识。
pub const TIER_SEVEN_DAY_FABLE: &str = "seven_day_fable";
pub const TIER_SEVEN_DAY_OPUS: &str = "seven_day_opus";
pub const TIER_SEVEN_DAY_SONNET: &str = "seven_day_sonnet";

/// Coding Plan（Kimi / MiniMax）的周窗口 tier 名。与 `coding_plan::query_*`
/// 写入、tray 渲染、commands::provider 扁平化三处共用同一标识。
pub const TIER_WEEKLY_LIMIT: &str = "weekly_limit";

/// 月窗口 tier 名。火山方舟 Agent Plan / Coding Plan 有 5h / 周 / 月 三个展示
/// 窗口（Kimi / MiniMax 只有 5h + 周），月窗口共用此标识；前端 `TIER_I18N_KEYS`
/// 映射到 `subscription.monthly`。
pub const TIER_MONTHLY: &str = "monthly";

/// Codex 免费方案的 30 天（月）滚动窗口 tier 名。付费方案的次要窗口是 7 天
/// (`seven_day`)，免费方案则是 30 天。由 `window_seconds_to_tier_name` 产出、
/// tray 的月分组渲染、前端 `TIER_I18N_KEYS` 映射到 `subscription.thirtyDay`
/// 三处共用同一标识。见 #3651。
pub const TIER_THIRTY_DAY: &str = "30_day";

/// Grok credit 额度窗口的兜底 tier 名。Grok 账单接口只返回一个 credit 用量
/// 窗口，`subscription_grok::tier_name_for_reset` 按重置距离优先映射到
/// `weekly_limit` / `monthly`，两者都不匹配时用此标识；前端 `TIER_I18N_KEYS`
/// 映射到 `subscription.credits`，tray 归入 "c" 分组。
pub const TIER_CREDITS: &str = "credits";

/// Gemini 用量分组名称（按模型而非时间窗口）。`classify_gemini_model` 输出。
pub const TIER_GEMINI_PRO: &str = "gemini_pro";
pub const TIER_GEMINI_FLASH: &str = "gemini_flash";
pub const TIER_GEMINI_FLASH_LITE: &str = "gemini_flash_lite";

/// Antigravity 专用：把配额聚合为两大模型族（对齐 Antigravity 官方 UI 的
/// "Gemini Models" / "Claude and GPT models" 分组），而不是逐模型一行。
/// 前端 `TIER_I18N_KEYS` 映射到 `subscription.geminiFamily` / `.claudeGptFamily`。
pub const TIER_GEMINI_FAMILY: &str = "gemini_family";
pub const TIER_CLAUDE_GPT_FAMILY: &str = "claude_gpt_family";

/// 当前套餐 tier 的前缀（`current_plan:<套餐名>`）。
///
/// Antigravity 免费档没有 `retrieveUserQuota` 的许可（实测 403 SUBSCRIPTION_REQUIRED），
/// 额度信息降级为 loadCodeAssist 的 `currentTier` —— 只有套餐名、没有百分比。前缀形式
/// 让前端能认出这是"套餐名"而不是未知窗口，同时对具体套餐名保持开放（上游可改）。
pub const TIER_CURRENT_PLAN_PREFIX: &str = "current_plan:";

const KNOWN_TIERS: &[&str] = &[
    TIER_FIVE_HOUR,
    TIER_SEVEN_DAY,
    TIER_SEVEN_DAY_FABLE,
    TIER_SEVEN_DAY_OPUS,
    TIER_SEVEN_DAY_SONNET,
];

/// 查询 Claude 官方订阅额度
///
/// 瞬时传输失败（网络/超时/读体中断）返回 `Err`（前端 reject → retry + 保留上次
/// 成功值）；确定性失败（鉴权/非 2xx/响应体非法 JSON）返回 `Ok(success:false)`。
/// codex/gemini 两个查询函数遵守同一约定。
async fn query_claude_quota(access_token: &str) -> Result<SubscriptionQuota, String> {
    let client = crate::proxy::http_client::get();

    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error: {e}")),
    };

    let status = resp.status();

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(SubscriptionQuota::error(
            "claude",
            CredentialStatus::Expired,
            format!("Authentication failed (HTTP {status}). Please re-login with Claude CLI."),
        ));
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Ok(SubscriptionQuota::error(
            "claude",
            CredentialStatus::Valid,
            format!("API error (HTTP {status}): {body}"),
        ));
    }

    // 先 bytes() 再解析：读体失败（超时/连接中断）是瞬时 → Err；拿到完整响应体
    // 后解析失败才是确定性。reqwest 的 json() 把读体错误也包成 decode，无法区分。
    let raw = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Err(format!("Failed to read API response: {e}")),
    };
    let body: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => {
            return Ok(SubscriptionQuota::error(
                "claude",
                CredentialStatus::Valid,
                format!("Failed to parse API response: {e}"),
            ));
        }
    };

    Ok(parse_claude_quota(&body))
}

/// 兼容旧顶层窗口与新版模型专属周限额，保持查询、缓存和 UI 共用 QuotaTier。
fn parse_claude_quota(body: &serde_json::Value) -> SubscriptionQuota {
    // 解析已知的 tier 窗口
    let mut tiers = Vec::new();
    for &tier_name in KNOWN_TIERS {
        if let Some(window) = body.get(tier_name) {
            if let Ok(w) = serde_json::from_value::<ApiUsageWindow>(window.clone()) {
                if let Some(util) = w.utilization {
                    tiers.push(QuotaTier {
                        name: tier_name.to_string(),
                        utilization: util,
                        resets_at: w.resets_at,
                        used_value_usd: None,
                        max_value_usd: None,
                    });
                }
            }
        }
    }

    // 也解析未知窗口（API 可能返回新的窗口类型）
    if let Some(obj) = body.as_object() {
        for (key, value) in obj {
            if key == "extra_usage" || key == "limits" || KNOWN_TIERS.contains(&key.as_str()) {
                continue;
            }
            if let Ok(w) = serde_json::from_value::<ApiUsageWindow>(value.clone()) {
                if let Some(util) = w.utilization {
                    tiers.push(QuotaTier {
                        name: key.clone(),
                        utilization: util,
                        resets_at: w.resets_at,
                        used_value_usd: None,
                        max_value_usd: None,
                    });
                }
            }
        }
    }

    // 新版模型专属额度覆盖同名旧窗口。逐条解析，单个异常项目不影响其余额度。
    let mut scoped_tiers = HashSet::new();
    if let Some(limits) = body.get("limits").and_then(serde_json::Value::as_array) {
        for limit in limits {
            if limit.get("kind").and_then(serde_json::Value::as_str) != Some("weekly_scoped")
                || limit.get("group").and_then(serde_json::Value::as_str) != Some("weekly")
                // 不把特定使用场景的子限额合并进整个模型的周限额。
                || limit.pointer("/scope/surface").is_some_and(|v| !v.is_null())
            {
                continue;
            }
            let Some(model) = limit
                .pointer("/scope/model/display_name")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let tier_name = match model.trim().to_ascii_lowercase().as_str() {
                "fable" => TIER_SEVEN_DAY_FABLE,
                "opus" => TIER_SEVEN_DAY_OPUS,
                "sonnet" => TIER_SEVEN_DAY_SONNET,
                _ => continue,
            };
            let Ok(window) = serde_json::from_value::<ApiScopedUsageWindow>(limit.clone()) else {
                continue;
            };
            if !window.percent.is_finite()
                || window.percent < 0.0
                || !scoped_tiers.insert(tier_name)
            {
                continue;
            }
            // 与 Claude Code 一致：不按 is_active 过滤。0% / resets_at:null
            // 也可能是有效的模型额度；不存在的额度由接口省略。
            let tier = QuotaTier {
                name: tier_name.to_string(),
                utilization: window.percent,
                resets_at: window.resets_at,
                used_value_usd: None,
                max_value_usd: None,
            };
            if let Some(existing) = tiers.iter_mut().find(|t| t.name == tier_name) {
                *existing = tier;
            } else {
                tiers.push(tier);
            }
        }
    }
    tiers.sort_by_key(|tier| {
        KNOWN_TIERS
            .iter()
            .position(|&name| name == tier.name)
            .unwrap_or(KNOWN_TIERS.len())
    });

    // 解析超额使用
    let extra_usage = body.get("extra_usage").and_then(|v| {
        serde_json::from_value::<ApiExtraUsage>(v.clone())
            .ok()
            .map(|e| ExtraUsage {
                is_enabled: e.is_enabled.unwrap_or(false),
                monthly_limit: e.monthly_limit,
                used_credits: e.used_credits,
                utilization: e.utilization,
                currency: e.currency,
            })
    });

    SubscriptionQuota {
        tool: "claude".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers,
        extra_usage,
        error: None,
        queried_at: Some(now_millis()),
    }
}

// ── Codex 凭据读取 ──────────────────────────────────────

#[derive(Deserialize)]
struct CodexAuthJson {
    auth_mode: Option<String>,
    tokens: Option<CodexTokens>,
    last_refresh: Option<String>,
}

#[derive(Deserialize)]
struct CodexTokens {
    access_token: Option<String>,
    account_id: Option<String>,
}

/// (access_token, account_id, status, message)
type CodexCredentials = (
    Option<String>,
    Option<String>,
    CredentialStatus,
    Option<String>,
);

/// 读取 Codex OAuth 凭据
///
/// 按优先级尝试以下来源：
/// 1. macOS Keychain (service: "Codex Auth")
/// 2. 凭据文件 ~/.codex/auth.json
///
/// 仅 auth_mode == "chatgpt" (OAuth) 时有效，API key 模式不支持用量查询。
fn read_codex_credentials() -> CodexCredentials {
    #[cfg(target_os = "macos")]
    {
        if let Some(result) = read_codex_credentials_from_keychain() {
            return result;
        }
    }

    read_codex_credentials_from_file()
}

/// 从 macOS Keychain 读取 Codex 凭据
#[cfg(target_os = "macos")]
fn read_codex_credentials_from_keychain() -> Option<CodexCredentials> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "Codex Auth", "-w"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json_str = String::from_utf8(output.stdout).ok()?;
    let json_str = json_str.trim();
    if json_str.is_empty() {
        return None;
    }

    Some(parse_codex_credentials_json(json_str))
}

/// 从文件读取 Codex 凭据
fn read_codex_credentials_from_file() -> CodexCredentials {
    let auth_path = crate::codex_config::get_codex_auth_path();

    if !auth_path.exists() {
        return (None, None, CredentialStatus::NotFound, None);
    }

    let content = match std::fs::read_to_string(&auth_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to read Codex auth file: {e}")),
            );
        }
    };

    parse_codex_credentials_json(&content)
}

/// 解析 Codex 凭据 JSON（Keychain 和文件共用）
fn parse_codex_credentials_json(content: &str) -> CodexCredentials {
    let auth: CodexAuthJson = match serde_json::from_str(content) {
        Ok(a) => a,
        Err(e) => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse Codex auth JSON: {e}")),
            );
        }
    };

    // 仅 OAuth 模式有用量数据
    if auth.auth_mode.as_deref() != Some("chatgpt") {
        return (
            None,
            None,
            CredentialStatus::NotFound,
            Some("Codex not using OAuth mode".to_string()),
        );
    }

    let tokens = match auth.tokens {
        Some(t) => t,
        None => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some("No tokens in Codex auth".to_string()),
            );
        }
    };

    let access_token = match tokens.access_token {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some("access_token is empty or missing".to_string()),
            );
        }
    };

    // 检查 token 是否可能过期（距上次刷新 > 8 天）
    if let Some(ref last_refresh) = auth.last_refresh {
        if is_codex_token_stale(last_refresh) {
            return (
                Some(access_token),
                tokens.account_id,
                CredentialStatus::Expired,
                Some("Codex token may be stale (>8 days since last refresh)".to_string()),
            );
        }
    }

    (
        Some(access_token),
        tokens.account_id,
        CredentialStatus::Valid,
        None,
    )
}

/// 判断 Codex token 是否可能过期（Codex CLI 在 >8 天时自动刷新）
fn is_codex_token_stale(last_refresh: &str) -> bool {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(last_refresh) {
        let age_secs = now_secs.saturating_sub(dt.timestamp() as u64);
        age_secs > 8 * 24 * 3600
    } else {
        false
    }
}

// ── Codex API 查询 ──────────────────────────────────────

#[derive(Deserialize)]
struct CodexRateLimitWindow {
    used_percent: Option<f64>,
    limit_window_seconds: Option<i64>,
    reset_at: Option<i64>,
}

#[derive(Deserialize)]
struct CodexRateLimit {
    primary_window: Option<CodexRateLimitWindow>,
    secondary_window: Option<CodexRateLimitWindow>,
}

#[derive(Deserialize)]
struct CodexUsageResponse {
    rate_limit: Option<CodexRateLimit>,
}

/// 根据窗口秒数映射到 tier 名称（与 Claude 的命名兼容以复用前端 i18n）
fn window_seconds_to_tier_name(secs: i64) -> String {
    match secs {
        18000 => TIER_FIVE_HOUR.to_string(),
        604800 => TIER_SEVEN_DAY.to_string(),
        // Codex 免费方案的 30 天窗口。显式映射到常量，与 tray 月分组、前端
        // TIER_I18N_KEYS 保持同一标识（否则动态回退虽也得到 "30_day"，但字符串
        // 分散在多处、易和托盘/前端白名单脱节）。见 #3651。
        2_592_000 => TIER_THIRTY_DAY.to_string(),
        s => {
            let hours = s / 3600;
            if hours >= 24 {
                format!("{}_day", hours / 24)
            } else {
                format!("{}_hour", hours)
            }
        }
    }
}

/// Unix 时间戳（秒）转 ISO 8601 字符串
fn unix_ts_to_iso(ts: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.to_rfc3339())
}

/// 查询 Codex / ChatGPT 反代订阅额度
///
/// 参数化 `tool_label` 和 `expired_message` 让该函数可被两个调用点共用：
/// - `"codex"` + "Please re-login with Codex CLI."（CLI 凭据路径）
/// - `"codex_oauth"` + "Please re-login via OGG Switch."（OGG Switch 自管 OAuth 路径）
pub(crate) async fn query_codex_quota(
    access_token: &str,
    account_id: Option<&str>,
    tool_label: &str,
    expired_message: &str,
) -> Result<SubscriptionQuota, String> {
    let client = crate::proxy::http_client::get();

    let mut req = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("User-Agent", "codex-cli")
        .header("Accept", "application/json");

    if let Some(id) = account_id {
        req = req.header("ChatGPT-Account-Id", id);
    }

    let resp = match req.timeout(std::time::Duration::from_secs(15)).send().await {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error: {e}")),
    };

    let status = resp.status();

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(SubscriptionQuota::error(
            tool_label,
            CredentialStatus::Expired,
            format!("{expired_message} (HTTP {status})"),
        ));
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Ok(SubscriptionQuota::error(
            tool_label,
            CredentialStatus::Valid,
            format!("API error (HTTP {status}): {body}"),
        ));
    }

    let raw = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Err(format!("Failed to read API response: {e}")),
    };
    let body: CodexUsageResponse = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => {
            return Ok(SubscriptionQuota::error(
                tool_label,
                CredentialStatus::Valid,
                format!("Failed to parse API response: {e}"),
            ));
        }
    };

    let mut tiers = Vec::new();

    if let Some(rate_limit) = body.rate_limit {
        for window in [rate_limit.primary_window, rate_limit.secondary_window]
            .into_iter()
            .flatten()
        {
            if let Some(used) = window.used_percent {
                tiers.push(QuotaTier {
                    name: window
                        .limit_window_seconds
                        .map(window_seconds_to_tier_name)
                        .unwrap_or_else(|| "unknown".to_string()),
                    utilization: used,
                    resets_at: window.reset_at.and_then(unix_ts_to_iso),
                    used_value_usd: None,
                    max_value_usd: None,
                });
            }
        }
    }

    Ok(SubscriptionQuota {
        tool: tool_label.to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers,
        extra_usage: None,
        error: None,
        queried_at: Some(now_millis()),
    })
}

// ── Gemini 凭据读取 ──────────────────────────────────────

/// Gemini OAuth 凭据文件格式（~/.gemini/oauth_creds.json）
#[derive(Deserialize)]
struct GeminiOAuthCredsFile {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expiry_date: Option<i64>, // 毫秒时间戳
}

/// (access_token, refresh_token, status, message)
type GeminiCredentials = (
    Option<String>,
    Option<String>,
    CredentialStatus,
    Option<String>,
);

/// 读取 Gemini OAuth 凭据
///
/// 按优先级尝试以下来源：
/// 1. macOS Keychain (service: "gemini-cli-oauth", account: "main-account")
/// 2. 凭据文件 ~/.gemini/oauth_creds.json（遗留格式）
///
/// 仅 OAuth 认证模式（`oauth-personal`）有效；API key 模式无法查询官方用量。
fn read_gemini_credentials() -> GeminiCredentials {
    #[cfg(target_os = "macos")]
    {
        if let Some(result) = read_gemini_credentials_from_keychain() {
            return result;
        }
    }

    read_gemini_credentials_from_file()
}

/// 从 macOS Keychain 读取 Gemini 凭据
#[cfg(target_os = "macos")]
fn read_gemini_credentials_from_keychain() -> Option<GeminiCredentials> {
    let output = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "gemini-cli-oauth",
            "-a",
            "main-account",
            "-w",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json_str = String::from_utf8(output.stdout).ok()?;
    let json_str = json_str.trim();
    if json_str.is_empty() {
        return None;
    }

    Some(parse_gemini_keychain_json(json_str))
}

/// 解析 Keychain 格式的 Gemini 凭据
///
/// Keychain 格式（keytar）：
/// ```json
/// { "token": { "accessToken": "...", "refreshToken": "...", "expiresAt": 1234 }, "updatedAt": ... }
/// ```
#[cfg(target_os = "macos")]
fn parse_gemini_keychain_json(content: &str) -> GeminiCredentials {
    let parsed: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(e) => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse Gemini keychain JSON: {e}")),
            )
        }
    };

    let token = match parsed.get("token") {
        Some(t) => t,
        None => {
            // Keychain 中可能是扁平格式，尝试文件格式解析
            return parse_gemini_file_json(content);
        }
    };

    let access_token = token
        .get("accessToken")
        .and_then(|v| v.as_str())
        .map(String::from);
    let refresh_token = token
        .get("refreshToken")
        .and_then(|v| v.as_str())
        .map(String::from);
    let expires_at = token.get("expiresAt").and_then(|v| v.as_i64());

    match access_token {
        Some(at) if !at.is_empty() => {
            // expiresAt 是毫秒时间戳
            if let Some(exp_ms) = expires_at {
                if exp_ms < now_millis() {
                    return (
                        Some(at),
                        refresh_token,
                        CredentialStatus::Expired,
                        Some("Gemini access token has expired".to_string()),
                    );
                }
            }
            (Some(at), refresh_token, CredentialStatus::Valid, None)
        }
        _ => (
            None,
            refresh_token,
            CredentialStatus::ParseError,
            Some("accessToken is empty or missing".to_string()),
        ),
    }
}

/// 从文件读取 Gemini 凭据
fn read_gemini_credentials_from_file() -> GeminiCredentials {
    let cred_path = crate::gemini_config::get_gemini_dir().join("oauth_creds.json");
    if !cred_path.exists() {
        return (None, None, CredentialStatus::NotFound, None);
    }

    let content = match std::fs::read_to_string(&cred_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to read Gemini credentials: {e}")),
            )
        }
    };

    parse_gemini_file_json(&content)
}

/// 解析文件格式的 Gemini 凭据
///
/// 文件格式（oauth_creds.json）：
/// ```json
/// { "access_token": "...", "refresh_token": "...", "expiry_date": 1234 }
/// ```
fn parse_gemini_file_json(content: &str) -> GeminiCredentials {
    let creds: GeminiOAuthCredsFile = match serde_json::from_str(content) {
        Ok(c) => c,
        Err(e) => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse Gemini credentials: {e}")),
            )
        }
    };

    let access_token = match creds.access_token {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                None,
                creds.refresh_token,
                CredentialStatus::ParseError,
                Some("access_token is empty or missing".to_string()),
            )
        }
    };

    // expiry_date 是毫秒时间戳
    if let Some(exp_ms) = creds.expiry_date {
        if exp_ms < now_millis() {
            return (
                Some(access_token),
                creds.refresh_token,
                CredentialStatus::Expired,
                Some("Gemini access token has expired".to_string()),
            );
        }
    }

    (
        Some(access_token),
        creds.refresh_token,
        CredentialStatus::Valid,
        None,
    )
}

// ── Gemini Token 刷新 ──────────────────────────────────────

/// Gemini OAuth Client 凭据（公开值，来自 Gemini CLI 源码 google-gemini/gemini-cli）
const GEMINI_OAUTH_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
const GEMINI_OAUTH_CLIENT_SECRET: &str = "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";

/// 使用 refresh_token 刷新 Gemini access token
///
/// Google OAuth access_token 仅有 ~1h 有效期，需要定期用 refresh_token 刷新。
/// refresh_token 本身不过期（除非用户撤销授权）。
async fn refresh_gemini_token(refresh_token: &str) -> Option<String> {
    let client = crate::proxy::http_client::get();

    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", GEMINI_OAUTH_CLIENT_ID),
            ("client_secret", GEMINI_OAUTH_CLIENT_SECRET),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let body: serde_json::Value = resp.json().await.ok()?;
    body.get("access_token")?.as_str().map(String::from)
}

// ── Gemini API 查询 ──────────────────────────────────────

/// loadCodeAssist 响应
#[derive(Deserialize)]
struct GeminiLoadCodeAssistResponse {
    #[serde(rename = "cloudaicompanionProject")]
    cloudaicompanion_project: Option<serde_json::Value>,
}

/// 配额 bucket
#[derive(Deserialize)]
struct GeminiBucketInfo {
    #[serde(rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
}

/// retrieveUserQuota 响应
#[derive(Deserialize)]
struct GeminiQuotaResponse {
    buckets: Option<Vec<GeminiBucketInfo>>,
}

/// `v1internal:fetchAvailableModels` 响应：`models` 是 modelId → 信息表，
/// 配额在 `quotaInfo` 里。作为 `retrieveUserQuota` 被许可墙挡住时的兜底数据源
/// （2026-09-24 实测：quota 端点对免费档 403 SUBSCRIPTION_REQUIRED，UA 无关；
/// fetchAvailableModels 带 Antigravity UA 可用，数据为真实周窗口）。
#[derive(Deserialize)]
struct CloudCodeQuotaInfo {
    #[serde(rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
}

#[derive(Deserialize)]
struct CloudCodeModelInfo {
    /// 服务端内部模型（如 `chat_*`）也带 quotaInfo，但不属于用户可见配额，解析时跳过。
    #[serde(rename = "isInternal")]
    is_internal: Option<bool>,
    #[serde(rename = "quotaInfo")]
    quota_info: Option<CloudCodeQuotaInfo>,
}

#[derive(Deserialize)]
struct CloudCodeAvailableModelsResponse {
    #[serde(default)]
    models: HashMap<String, CloudCodeModelInfo>,
}

/// 从 loadCodeAssist 响应中提取项目 ID
fn extract_project_id(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(obj) => obj
            .get("id")
            .or_else(|| obj.get("projectId"))
            .and_then(|v| v.as_str())
            .map(String::from),
        _ => None,
    }
}

/// 将 Gemini 模型 ID 分类为 Pro / Flash / Flash Lite
fn classify_gemini_model(model_id: &str) -> &str {
    if model_id.contains("flash-lite") {
        TIER_GEMINI_FLASH_LITE
    } else if model_id.contains("flash") {
        TIER_GEMINI_FLASH
    } else if model_id.contains("pro") {
        TIER_GEMINI_PRO
    } else {
        model_id
    }
}

/// Cloud Code（`v1internal`）额度查询目标。
///
/// Gemini CLI 与 Antigravity 共用这套接口，差异只有这几处参数：
/// - **base_url**：Gemini CLI 走 `https://cloudcode-pa.googleapis.com`；agy 自己的流量
///   实测走 `https://daily-cloudcode-pa.googleapis.com`（见 `ANTIGRAVITY_CLOUDCODE_BASE`）。
/// - **ide_type / plugin_type**：是 `ClientMetadata` 的 proto 枚举名。`ANTIGRAVITY` 与
///   `GEMINI_CLI` 都是 `IdeType` 的成员（枚举表见 agy 二进制内嵌的 proto 描述符）。
/// - **relogin_hint**：401/403 文案里让用户去重新登录的那个 CLI。
/// - **fallback_plan_name**：`retrieveUserQuota` 403（无许可）时套餐兜底 tier 的显示名
///   （如 "Antigravity"）；Gemini 不需要（它的 retrieveUserQuota 正常可用）。
/// - **user_agent**：Cloud Code 上游按 UA 过滤客户端（实测 UA 无 Antigravity 标识时
///   `fetchAvailableModels` 直接 403 PERMISSION_DENIED，见
///   [`ANTIGRAVITY_CLOUDCODE_USER_AGENT`]）。Gemini CLI 的请求不带该头（多年实测无需）。
/// - **family_tiers**：true（Antigravity）时配额聚合为 Gemini / Claude+GPT 两大模型族
///   （对齐 Antigravity 官方 UI 分组）；false（Gemini CLI）维持 pro/flash/flash-lite
///   逐类细分。
///
/// `base_url` 带 scheme 是为了单测能把它指向本地 mock server（冒烟测试无法打真接口）。
struct CloudCodeTarget<'a> {
    tool: &'a str,
    base_url: &'a str,
    ide_type: &'a str,
    plugin_type: &'a str,
    relogin_hint: &'a str,
    fallback_plan_name: Option<&'a str>,
    user_agent: Option<&'a str>,
    family_tiers: bool,
}

/// 查询 Gemini 官方订阅额度
///
/// 两步 API 调用：
/// 1. loadCodeAssist → 获取 cloudaicompanionProject
/// 2. retrieveUserQuota → 获取按模型分桶的配额数据
async fn query_gemini_quota(access_token: &str) -> Result<SubscriptionQuota, String> {
    query_cloudcode_quota(
        &CloudCodeTarget {
            tool: "gemini",
            base_url: "https://cloudcode-pa.googleapis.com",
            ide_type: "GEMINI_CLI",
            plugin_type: "GEMINI",
            relogin_hint: "Gemini CLI",
            fallback_plan_name: None,
            user_agent: None,
            family_tiers: false,
        },
        access_token,
    )
    .await
}

/// 查询 Antigravity（agy CLI）官方订阅额度。
///
/// 与 Gemini 同一条 Cloud Code 链路，只是主机是 agy 自己用的 `daily-` 前缀、
/// `ideType` 是 `ANTIGRAVITY`；响应形态相同（`buckets[].modelId/remainingFraction/resetTime`），
/// 所以复用同一份解析与分类。
async fn query_antigravity_quota(access_token: &str) -> Result<SubscriptionQuota, String> {
    query_cloudcode_quota(
        &CloudCodeTarget {
            tool: "antigravity",
            base_url: ANTIGRAVITY_CLOUDCODE_BASE,
            ide_type: "ANTIGRAVITY",
            // Antigravity 是 Cloud Code 家族的编辑器插件；`PluginType` 枚举里没有
            // ANTIGRAVITY 成员（PLUGIN_UNSPECIFIED / CLOUD_CODE / GEMINI / …），
            // 取 `CLOUD_CODE`。该字段只影响服务端统计归类。
            plugin_type: "CLOUD_CODE",
            relogin_hint: "agy",
            // 免费档无 retrieveUserQuota 许可（403 SUBSCRIPTION_REQUIRED）时的套餐名
            fallback_plan_name: Some("Antigravity"),
            user_agent: Some(ANTIGRAVITY_CLOUDCODE_USER_AGENT),
            family_tiers: true,
        },
        access_token,
    )
    .await
}

/// 从 Google 风格的错误响应里提取人话原因（`error.message` + `ErrorInfo.reason`），
/// 供非 2xx 分支透出——比塞整段 JSON 进错误详情可读得多。
fn upstream_error_detail(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        // 非 JSON：截断原文
        let preview: String = body.trim().chars().take(200).collect();
        return if preview.is_empty() {
            String::new()
        } else {
            format!(": {preview}")
        };
    };
    let error = &value["error"];
    let message = error["message"].as_str().unwrap_or_default().trim();
    let reason = error["details"]
        .as_array()
        .and_then(|details| {
            details.iter().find_map(|d| {
                let reason = d["reason"].as_str()?;
                (d["@type"]
                    .as_str()
                    .unwrap_or_default()
                    .ends_with("ErrorInfo"))
                .then(|| reason.to_string())
            })
        })
        .unwrap_or_default();

    match (reason.is_empty(), message.is_empty()) {
        (false, false) => format!(": [{reason}] {message}"),
        (false, true) => format!(": [{reason}]"),
        (true, false) => format!(": {message}"),
        (true, true) => String::new(),
    }
}

/// 从 loadCodeAssist 的响应里取当前套餐，构成一个"套餐名"tier。
///
/// 实测响应（Antigravity 免费档）：
/// `{"currentTier": {"id": "free-tier", "name": "Antigravity", "description": …}, …}`
/// agy 终端横幅里的 `Antigravity Starter Quota` 就是这条信息。它没有百分比/窗口数据
/// ——那类数据只有 `retrieveUserQuota` 有，而免费档没有该 API 的许可——所以 tier 名
/// 用 `subscription` 前缀的专用键，前端 TIER_I18N_KEYS 给它一个"当前套餐"标签，
/// utilization 固定 0（颜色恒绿，不参与已用/剩余标注）。
fn cloudcode_tier_fallback(load_value: &serde_json::Value) -> Option<QuotaTier> {
    let tier = &load_value["currentTier"];
    // name 优先（如 "Antigravity"）；缺失时退到 id（如 "free-tier"）
    let name = tier["name"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| tier["id"].as_str().map(str::trim).filter(|s| !s.is_empty()))
        .map(String::from)?;
    Some(QuotaTier {
        name: format!("{TIER_CURRENT_PLAN_PREFIX}{name}"),
        utilization: 0.0,
        resets_at: None,
        used_value_usd: None,
        max_value_usd: None,
    })
}

/// Cloud Code `v1internal` 额度查询（loadCodeAssist → retrieveUserQuota）。
async fn query_cloudcode_quota(
    target: &CloudCodeTarget<'_>,
    access_token: &str,
) -> Result<SubscriptionQuota, String> {
    let client = crate::proxy::http_client::get();
    let tool = target.tool;

    // ── Step 1: loadCodeAssist 获取项目 ID ──
    let mut load_req = client
        .post(format!("{}/v1internal:loadCodeAssist", target.base_url))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json");
    if let Some(ua) = target.user_agent {
        load_req = load_req.header("User-Agent", ua);
    }
    let load_resp = load_req
        .json(&serde_json::json!({
            "metadata": {
                "ideType": target.ide_type,
                "pluginType": target.plugin_type
            }
        }))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;

    let load_resp = match load_resp {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error (loadCodeAssist): {e}")),
    };

    let load_status = load_resp.status();
    if load_status == reqwest::StatusCode::UNAUTHORIZED {
        return Ok(SubscriptionQuota::error(
            tool,
            CredentialStatus::Expired,
            format!(
                "Authentication failed (HTTP {load_status}). Please re-login with {}.",
                target.relogin_hint
            ),
        ));
    }
    if !load_status.is_success() {
        // 403/其它：带上游原因（reason + message），不做"过期"推断——
        // token 是否有效由 401 决定，403 多半是许可/套餐/权限，重新登录解决不了。
        let body = load_resp.text().await.unwrap_or_default();
        let detail = upstream_error_detail(&body);
        return Ok(SubscriptionQuota::error(
            tool,
            CredentialStatus::Valid,
            format!("loadCodeAssist failed (HTTP {load_status}){detail}"),
        ));
    }

    let load_raw = match load_resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Err(format!("Failed to read loadCodeAssist response: {e}")),
    };
    // 完整 body 先留一份：Antigravity 的套餐信息（currentTier）在这里，免费档
    // `retrieveUserQuota` 会 403，届时用它兜底而不是报"会话过期"。
    let load_value: serde_json::Value = match serde_json::from_slice(&load_raw) {
        Ok(v) => v,
        Err(e) => {
            return Ok(SubscriptionQuota::error(
                tool,
                CredentialStatus::Valid,
                format!("Failed to parse loadCodeAssist response: {e}"),
            ))
        }
    };
    let load_body: GeminiLoadCodeAssistResponse = match serde_json::from_value(load_value.clone()) {
        Ok(v) => v,
        Err(e) => {
            return Ok(SubscriptionQuota::error(
                tool,
                CredentialStatus::Valid,
                format!("Failed to parse loadCodeAssist response: {e}"),
            ))
        }
    };

    let project_id = load_body
        .cloudaicompanion_project
        .as_ref()
        .and_then(extract_project_id);

    // ── Step 2: retrieveUserQuota 获取配额 ──
    let mut quota_body = serde_json::json!({});
    if let Some(ref pid) = project_id {
        quota_body["project"] = serde_json::Value::String(pid.clone());
    }

    let mut quota_req = client
        .post(format!("{}/v1internal:retrieveUserQuota", target.base_url))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json");
    if let Some(ua) = target.user_agent {
        quota_req = quota_req.header("User-Agent", ua);
    }
    let quota_resp = quota_req
        .json(&quota_body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;

    let quota_resp = match quota_resp {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error (retrieveUserQuota): {e}")),
    };

    let quota_status = quota_resp.status();
    if quota_status == reqwest::StatusCode::UNAUTHORIZED {
        // 只有 401 才是"登录态失效"：403 往往是许可/套餐问题（实测 Antigravity 免费档
        // 会回 403 + SUBSCRIPTION_REQUIRED，账号明明是登录着的），映射成 Expired 会
        // 误导用户去重新登录。403 落到下面"带响应体"的错误分支。
        return Ok(SubscriptionQuota::error(
            tool,
            CredentialStatus::Expired,
            format!(
                "Authentication failed (HTTP {quota_status}). Please re-login with {}.",
                target.relogin_hint
            ),
        ));
    }
    if !quota_status.is_success() {
        // 实测（2026-09-24，Antigravity 免费档 / Antigravity Starter Quota）：
        // 403 + SUBSCRIPTION_REQUIRED —— 账号登录正常（loadCodeAssist 200、
        // currentTier=free-tier），但该账号没有 retrieveUserQuota 的许可。
        // 先试 fetchAvailableModels 兜底（quota 端点被许可墙挡、UA 无关，见
        // `try_query_available_models_quota` 的探针结论）；仍拿不到才降级。
        let body = quota_resp.text().await.unwrap_or_default();
        let detail = upstream_error_detail(&body);

        if target.user_agent.is_some() {
            if let Some(tiers) =
                try_query_available_models_quota(&client, target, access_token, &quota_body).await
            {
                log::info!(
                    "retrieveUserQuota 失败（HTTP {quota_status}），fetchAvailableModels 兜底得到 {} 个模型分类",
                    tiers.len()
                );
                return Ok(SubscriptionQuota {
                    tool: tool.to_string(),
                    credential_status: CredentialStatus::Valid,
                    credential_message: None,
                    success: true,
                    tiers,
                    extra_usage: None,
                    error: None,
                    queried_at: Some(now_millis()),
                });
            }
        }

        // agy 自己的「Antigravity Starter Quota」横幅取自 loadCodeAssist 的
        // currentTier，这里降级用它兜底：显示当前套餐而不是报错/误报过期。
        // 兜底 tier 名可由调用方传入（上游响应形态变化时不至于整条失败）。
        if quota_status == reqwest::StatusCode::FORBIDDEN
            && detail.contains("SUBSCRIPTION_REQUIRED")
        {
            if let Some(mut tier) = cloudcode_tier_fallback(&load_value) {
                log::info!(
                    "retrieveUserQuota 返回 403（无许可），用 loadCodeAssist 的 currentTier 兜底: {}",
                    tier.name
                );
                if let Some(name) = target.fallback_plan_name {
                    tier.name = format!("{TIER_CURRENT_PLAN_PREFIX}{name}");
                }
                return Ok(SubscriptionQuota {
                    tool: tool.to_string(),
                    credential_status: CredentialStatus::Valid,
                    credential_message: None,
                    success: true,
                    tiers: vec![tier],
                    extra_usage: None,
                    error: None,
                    queried_at: Some(now_millis()),
                });
            }
            // loadCodeAssist 连 currentTier 都没给：用工具名兜底构造一个套餐 tier，
            // 让用户至少看到「套餐 Antigravity」而不是一屏裸 403。
            log::info!(
                "retrieveUserQuota 403 且 loadCodeAssist 无 currentTier，用工具名兜底套餐 tier"
            );
            return Ok(SubscriptionQuota {
                tool: tool.to_string(),
                credential_status: CredentialStatus::Valid,
                credential_message: None,
                success: true,
                tiers: vec![QuotaTier {
                    name: format!(
                        "{TIER_CURRENT_PLAN_PREFIX}{}",
                        target.fallback_plan_name.unwrap_or_else(|| {
                            if tool == "antigravity" {
                                "Antigravity"
                            } else {
                                tool
                            }
                        })
                    ),
                    utilization: 0.0,
                    resets_at: None,
                    used_value_usd: None,
                    max_value_usd: None,
                }],
                extra_usage: None,
                error: None,
                queried_at: Some(now_millis()),
            });
        }
        return Ok(SubscriptionQuota::error(
            tool,
            CredentialStatus::Valid,
            format!("retrieveUserQuota failed (HTTP {quota_status}){detail}"),
        ));
    }

    let quota_raw = match quota_resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Err(format!("Failed to read quota response: {e}")),
    };
    let quota_data: GeminiQuotaResponse = match serde_json::from_slice(&quota_raw) {
        Ok(v) => v,
        Err(e) => {
            return Ok(SubscriptionQuota::error(
                tool,
                CredentialStatus::Valid,
                format!("Failed to parse quota response: {e}"),
            ))
        }
    };

    // ── 按模型分类汇总，每类取最低 remainingFraction ──
    // Antigravity 的 buckets 会混入 `chat_*` 等服务端内部模型，展示前按白名单滤掉
    // （modelId 缺失的桶保留，维持 Gemini 路径的既有行为）。
    let buckets = quota_data
        .buckets
        .unwrap_or_default()
        .into_iter()
        .filter(|bucket| {
            bucket
                .model_id
                .as_deref()
                .is_none_or(is_user_facing_cloudcode_model)
        })
        .collect();
    let tiers = build_cloudcode_model_tiers(buckets, target.family_tiers);

    Ok(SubscriptionQuota {
        tool: tool.to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers,
        extra_usage: None,
        error: None,
        queried_at: Some(now_millis()),
    })
}

// ── Antigravity（agy）凭据与额度 ──────────────────────────

/// Antigravity 两大模型族归类（对齐其官方 UI 的 "Gemini Models" /
/// "Claude and GPT models" 分组）。调用前已经过
/// `is_user_facing_cloudcode_model` 白名单过滤，只会有这几个前缀。
fn cloudcode_model_family(model_id: &str) -> &'static str {
    if model_id.starts_with("claude") || model_id.starts_with("gpt") {
        TIER_CLAUDE_GPT_FAMILY
    } else {
        TIER_GEMINI_FAMILY
    }
}

/// 把按模型分桶的配额聚合成展示 tiers。`family_tiers`（Antigravity）按两大模型族
/// 聚合，否则（Gemini CLI）按 `classify_gemini_model` 细分 pro/flash/flash-lite；
/// 每类取最低 remainingFraction（最受限的窗口），remainingFraction → utilization
/// （已用百分比）。`retrieveUserQuota` 的 buckets 与 `fetchAvailableModels` 兜底
/// 共用这一份聚合。
fn build_cloudcode_model_tiers(
    buckets: Vec<GeminiBucketInfo>,
    family_tiers: bool,
) -> Vec<QuotaTier> {
    let mut category_map: HashMap<String, (f64, Option<String>)> = HashMap::new();
    for bucket in buckets {
        let model_id = bucket.model_id.as_deref().unwrap_or("unknown");
        let category = if family_tiers {
            cloudcode_model_family(model_id).to_string()
        } else {
            classify_gemini_model(model_id).to_string()
        };
        let remaining = bucket.remaining_fraction.unwrap_or(1.0).clamp(0.0, 1.0);

        let entry = category_map
            .entry(category)
            .or_insert((remaining, bucket.reset_time.clone()));
        if remaining < entry.0 {
            entry.0 = remaining;
            if bucket.reset_time.is_some() {
                entry.1.clone_from(&bucket.reset_time);
            }
        }
    }

    let sort_order = |name: &str| -> usize {
        match name {
            TIER_GEMINI_FAMILY | TIER_GEMINI_PRO => 0,
            TIER_CLAUDE_GPT_FAMILY | TIER_GEMINI_FLASH => 1,
            TIER_GEMINI_FLASH_LITE => 2,
            _ => 3,
        }
    };

    let mut tiers: Vec<QuotaTier> = category_map
        .into_iter()
        .map(|(name, (remaining, reset_time))| QuotaTier {
            name,
            utilization: (1.0 - remaining) * 100.0,
            resets_at: reset_time,
            used_value_usd: None,
            max_value_usd: None,
        })
        .collect();

    tiers.sort_by_key(|t| sort_order(&t.name));
    tiers
}

/// 用户可见的 Cloud Code 模型 id 前缀白名单（Antigravity-Manager 同款口径）。
/// 服务端还会下发 `chat_20706` 这类内部模型——它们未必带 `isInternal` 标记，
/// 单靠该字段过滤不住，白名单一并兜住。
fn is_user_facing_cloudcode_model(model_id: &str) -> bool {
    const PREFIXES: [&str; 5] = ["gemini", "claude", "gpt", "image", "imagen"];
    PREFIXES.iter().any(|p| model_id.starts_with(p))
}

/// `retrieveUserQuota` 失败时的兜底数据源：`v1internal:fetchAvailableModels`。
///
/// 2026-09-24 探针结论（真实 agy 凭据 × daily/prod/sandbox 三主机，探针已删）：
/// 无 Antigravity UA 时 quota 双端点一律 403（「summary 绕过 403」的立项假设不成立），
/// 带 [`ANTIGRAVITY_CLOUDCODE_USER_AGENT`] 后 `retrieveUserQuota` 主路径即恢复 200；
/// 本兜底覆盖 quota 仍失败的情形（个别账号/套餐状态），`fetchAvailableModels`
/// 返回 per-model `models.<id>.quotaInfo.{remainingFraction, resetTime}`。
///
/// 解析为与 buckets 同形的数据后复用 [`build_cloudcode_model_tiers`] 聚合；
/// 任何一步拿不到数据都返回 `None`（调用方继续走原有 403 兜底/报错路径）。
async fn try_query_available_models_quota(
    client: &reqwest::Client,
    target: &CloudCodeTarget<'_>,
    access_token: &str,
    quota_body: &serde_json::Value,
) -> Option<Vec<QuotaTier>> {
    let mut request = client
        .post(format!(
            "{}/v1internal:fetchAvailableModels",
            target.base_url
        ))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json");
    if let Some(ua) = target.user_agent {
        request = request.header("User-Agent", ua);
    }
    let resp = request
        .json(quota_body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        log::info!("fetchAvailableModels 兜底失败（HTTP {}）", resp.status());
        return None;
    }
    let raw = resp.bytes().await.ok()?;
    let data: CloudCodeAvailableModelsResponse = serde_json::from_slice(&raw).ok()?;
    let buckets: Vec<GeminiBucketInfo> = data
        .models
        .into_iter()
        .filter(|(model_id, info)| {
            is_user_facing_cloudcode_model(model_id)
                && !info.is_internal.unwrap_or(false)
                && info.quota_info.is_some()
        })
        .map(|(model_id, info)| match info.quota_info {
            Some(quota) => GeminiBucketInfo {
                remaining_fraction: quota.remaining_fraction,
                reset_time: quota.reset_time,
                model_id: Some(model_id),
            },
            None => GeminiBucketInfo {
                remaining_fraction: None,
                reset_time: None,
                model_id: Some(model_id),
            },
        })
        .collect();
    if buckets.is_empty() {
        return None;
    }
    Some(build_cloudcode_model_tiers(buckets, target.family_tiers))
}

/// agy 自己流量走的 Cloud Code 主机（实测其 `cli.log`：`daily-cloudcode-pa.googleapis.com`）
const ANTIGRAVITY_CLOUDCODE_BASE: &str = "https://daily-cloudcode-pa.googleapis.com";

/// Cloud Code 上游按 UA 过滤客户端。2026-09-24 实测（真实 agy 凭据 × daily/prod/sandbox）：
/// reqwest 默认 UA 下 quota 请求 403（quota 端点报 SUBSCRIPTION_REQUIRED、
/// fetchAvailableModels 报 PERMISSION_DENIED）；带含 "Antigravity" 的 UA 后整条链路解锁
/// （loadCodeAssist 下发 project → retrieveUserQuota 200）。版本号取 Antigravity-Manager
/// 钳制的已知稳定值（`KNOWN_STABLE_VERSION = "4.3.0"`），格式仿其
/// `NATIVE_OAUTH_USER_AGENT`。
const ANTIGRAVITY_CLOUDCODE_USER_AGENT: &str = "vscode/1.100.0 (Antigravity/4.3.0)";

/// 读取 Antigravity 凭据（`(access_token, status, message)`，与其它 app 的读取器同形）。
///
/// 来源与优先级交给 `antigravity_config::agy_token_state`（**keyring 优先、token 文件兜底**，
/// 与 agy 自己的组合存储一致）：`~/.gemini/antigravity-cli/antigravity-oauth-token`
/// （`{access_token, token_type, refresh_token, expiry, email}`）与 Windows 凭据管理器
/// `gemini:antigravity`（**`{"token": {access_token, …, expiry}}` —— token 是嵌套对象**）。
///
/// **刻意不刷新 token**：agy 的 OAuth client 不是公开凭据，不能像 Gemini CLI 那样内嵌
/// （见 `GEMINI_OAUTH_CLIENT_ID` 的说明，以及仓库「不写入第三方 OAuth client 凭据」的约束）。
/// access token 过期时交给前端提示用户跑一次 `agy` 重新登录。
fn read_antigravity_credentials() -> (Option<String>, CredentialStatus, Option<String>) {
    read_antigravity_credentials_with(crate::antigravity_config::credential_manager::find_matching)
}

/// `read_antigravity_credentials` 的可注入版本：keyring 读取由调用方提供，
/// 单测因此不必触碰真实系统凭据条目（真实实现是 `credential_manager::find_matching`）。
fn read_antigravity_credentials_with(
    keyring: impl Fn() -> Result<
        Vec<crate::antigravity_config::CredentialSnapshot>,
        crate::error::AppError,
    >,
) -> (Option<String>, CredentialStatus, Option<String>) {
    use crate::antigravity_config::AgyTokenState;

    match crate::antigravity_config::agy_token_state(keyring) {
        AgyTokenState::Usable(token) => (Some(token.token), CredentialStatus::Valid, None),
        // 全都过期：把最晚的一枚交回去，调用方仍会试一次（CLI 可能刚在别处刷新过）。
        // 读取过程中的问题（另一处凭据损坏等）附在提示里，别丢信息。
        AgyTokenState::Expired { token, diagnostic } => {
            let message = match diagnostic {
                Some(detail) => format!("Antigravity access token has expired ({detail})"),
                None => "Antigravity access token has expired".to_string(),
            };
            (Some(token.token), CredentialStatus::Expired, Some(message))
        }
        // 读到了但解析不出来 → 凭据损坏；一处都没有 → 没登录。
        AgyTokenState::Missing { diagnostic } => match diagnostic {
            Some(message) => (None, CredentialStatus::ParseError, Some(message)),
            None => (None, CredentialStatus::NotFound, None),
        },
    }
}

// ── 入口函数 ──────────────────────────────────────────────

/// 查询指定 CLI 工具的官方订阅额度
///
/// 瞬时传输失败以 `Err` 传播（前端 reject → retry + 保留上次成功值）。Expired
/// 分支的"过期也试一把"重试同样用 `?` 传播瞬时错误——不能折叠成"已过期"，
/// 否则一次网络抖动会被误报成确定性的凭据过期。
pub async fn get_subscription_quota(tool: &str) -> Result<SubscriptionQuota, String> {
    match tool {
        "claude" => {
            let (token, status, message) = read_claude_credentials();

            match status {
                CredentialStatus::NotFound => Ok(SubscriptionQuota::not_found("claude")),
                CredentialStatus::ParseError => Ok(SubscriptionQuota::error(
                    "claude",
                    CredentialStatus::ParseError,
                    message.unwrap_or_else(|| "Failed to parse credentials".to_string()),
                )),
                CredentialStatus::Expired => {
                    // 即使过期也尝试调用 API（token 可能实际上仍有效）
                    if let Some(token) = token {
                        let result = query_claude_quota(&token).await?;
                        if result.success {
                            return Ok(result);
                        }
                    }
                    Ok(SubscriptionQuota::error(
                        "claude",
                        CredentialStatus::Expired,
                        message.unwrap_or_else(|| "OAuth token has expired".to_string()),
                    ))
                }
                CredentialStatus::Valid => {
                    let token = token.expect("token must be Some when status is Valid");
                    query_claude_quota(&token).await
                }
            }
        }
        "codex" => {
            let (token, account_id, status, message) = read_codex_credentials();

            match status {
                CredentialStatus::NotFound => Ok(SubscriptionQuota::not_found("codex")),
                CredentialStatus::ParseError => Ok(SubscriptionQuota::error(
                    "codex",
                    CredentialStatus::ParseError,
                    message.unwrap_or_else(|| "Failed to parse credentials".to_string()),
                )),
                CredentialStatus::Expired => {
                    // 即使可能过期也尝试调用 API
                    if let Some(token) = token {
                        let result = query_codex_quota(
                            &token,
                            account_id.as_deref(),
                            "codex",
                            "Authentication failed. Please re-login with Codex CLI.",
                        )
                        .await?;
                        if result.success {
                            return Ok(result);
                        }
                    }
                    Ok(SubscriptionQuota::error(
                        "codex",
                        CredentialStatus::Expired,
                        message.unwrap_or_else(|| "Codex OAuth token may be stale".to_string()),
                    ))
                }
                CredentialStatus::Valid => {
                    let token = token.expect("token must be Some when status is Valid");
                    query_codex_quota(
                        &token,
                        account_id.as_deref(),
                        "codex",
                        "Authentication failed. Please re-login with Codex CLI.",
                    )
                    .await
                }
            }
        }
        "gemini" => {
            let (token, refresh_token, status, message) = read_gemini_credentials();

            match status {
                CredentialStatus::NotFound => Ok(SubscriptionQuota::not_found("gemini")),
                CredentialStatus::ParseError => Ok(SubscriptionQuota::error(
                    "gemini",
                    CredentialStatus::ParseError,
                    message.unwrap_or_else(|| "Failed to parse credentials".to_string()),
                )),
                CredentialStatus::Expired => {
                    // Gemini access_token 仅 ~1h 有效，尝试用 refresh_token 刷新
                    if let Some(ref rt) = refresh_token {
                        if let Some(new_token) = refresh_gemini_token(rt).await {
                            return query_gemini_quota(&new_token).await;
                        }
                    }
                    // 刷新失败，尝试用旧 token
                    if let Some(ref token) = token {
                        let result = query_gemini_quota(token).await?;
                        if result.success {
                            return Ok(result);
                        }
                    }
                    Ok(SubscriptionQuota::error(
                        "gemini",
                        CredentialStatus::Expired,
                        message.unwrap_or_else(|| "Gemini OAuth token has expired".to_string()),
                    ))
                }
                CredentialStatus::Valid => {
                    let token = token.expect("token must be Some when status is Valid");
                    query_gemini_quota(&token).await
                }
            }
        }
        "grokbuild" => crate::services::subscription_grok::get_grok_subscription_quota().await,
        // Antigravity：额度与 Gemini 同族（Cloud Code v1internal），凭据取 agy 自己的
        // 登录态且**不刷新**（见 `read_antigravity_credentials`）。
        "antigravity" => {
            let (token, status, message) = read_antigravity_credentials();

            match status {
                CredentialStatus::NotFound => Ok(SubscriptionQuota::not_found("antigravity")),
                CredentialStatus::ParseError => Ok(SubscriptionQuota::error(
                    "antigravity",
                    CredentialStatus::ParseError,
                    message.unwrap_or_else(|| "Failed to parse credentials".to_string()),
                )),
                CredentialStatus::Expired => {
                    // 本地快照说过期，仍试一把：另一种凭据来源可能是新的
                    if let Some(token) = token {
                        let result = query_antigravity_quota(&token).await?;
                        if result.success {
                            return Ok(result);
                        }
                    }
                    Ok(SubscriptionQuota::error(
                        "antigravity",
                        CredentialStatus::Expired,
                        message
                            .unwrap_or_else(|| "Antigravity access token has expired".to_string()),
                    ))
                }
                CredentialStatus::Valid => {
                    let token = token.expect("token must be Some when status is Valid");
                    query_antigravity_quota(&token).await
                }
            }
        }
        _ => Ok(SubscriptionQuota::not_found(tool)),
    }
}

// ── 辅助函数 ──────────────────────────────────────────────

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scoped_limit(model: &str, percent: f64) -> serde_json::Value {
        serde_json::json!({
            "kind": "weekly_scoped",
            "group": "weekly",
            "percent": percent,
            "resets_at": "2026-09-12T00:00:00Z",
            "is_active": true,
            "scope": { "model": { "id": null, "display_name": model }, "surface": null }
        })
    }

    #[test]
    fn claude_quota_preserves_legacy_windows_and_extra_usage() {
        let quota = parse_claude_quota(&serde_json::json!({
            "five_hour": { "utilization": 12.0, "resets_at": "2026-09-09T15:00:00Z" },
            "seven_day": { "utilization": 25.0, "resets_at": null },
            "seven_day_opus": { "utilization": 8.0 },
            "seven_day_sonnet": null,
            "other_window": { "utilization": 4.0 },
            "extra_usage": { "is_enabled": true, "monthly_limit": 100.0,
                "used_credits": 9.0, "utilization": 9.0, "currency": "USD" }
        }));
        assert!(quota.success);
        assert_eq!(quota.tool, "claude");
        assert_eq!(
            quota
                .tiers
                .iter()
                .map(|t| (t.name.as_str(), t.utilization))
                .collect::<Vec<_>>(),
            vec![
                (TIER_FIVE_HOUR, 12.0),
                (TIER_SEVEN_DAY, 25.0),
                (TIER_SEVEN_DAY_OPUS, 8.0),
                ("other_window", 4.0)
            ]
        );
        assert_eq!(
            quota.tiers[0].resets_at.as_deref(),
            Some("2026-09-09T15:00:00Z")
        );
        let extra = quota.extra_usage.unwrap();
        assert!(extra.is_enabled);
        assert_eq!(extra.used_credits, Some(9.0));
        assert_eq!(extra.monthly_limit, Some(100.0));
        assert_eq!(extra.currency.as_deref(), Some("USD"));
    }

    #[test]
    fn claude_quota_adds_fable_from_limits_array() {
        let quota = parse_claude_quota(&serde_json::json!({
            "five_hour": { "utilization": 12.0 },
            "seven_day": { "utilization": 25.0 },
            "seven_day_opus": null,
            "seven_day_sonnet": null,
            "limits": [scoped_limit("Fable", 37.5)]
        }));
        assert_eq!(quota.tiers.len(), 3);
        let tier = &quota.tiers[2];
        assert_eq!(tier.name, TIER_SEVEN_DAY_FABLE);
        assert_eq!(tier.utilization, 37.5);
        assert_eq!(tier.resets_at.as_deref(), Some("2026-09-12T00:00:00Z"));
        // 前端与缓存使用同一份 camelCase 数据，无需额外字段。
        let serialized = serde_json::to_value(&quota).unwrap();
        assert_eq!(serialized["tiers"][2]["resetsAt"], "2026-09-12T00:00:00Z");
    }

    #[test]
    fn claude_quota_scoped_windows_override_legacy_and_deduplicate() {
        let mut fable = scoped_limit("  fAbLe  ", 0.0);
        fable["is_active"] = serde_json::json!(false);
        fable["resets_at"] = serde_json::Value::Null;
        let quota = parse_claude_quota(&serde_json::json!({
            "seven_day_fable": { "utilization": 80.0, "resets_at": "2026-09-11T00:00:00Z" },
            "seven_day_opus": { "utilization": 20.0 },
            "seven_day_sonnet": { "utilization": 30.0 },
            "limits": [scoped_limit("Sonnet", 5.0), fable, scoped_limit("Fable", 90.0), scoped_limit("Opus", 6.0)]
        }));
        assert_eq!(
            quota
                .tiers
                .iter()
                .map(|t| (t.name.as_str(), t.utilization))
                .collect::<Vec<_>>(),
            vec![
                (TIER_SEVEN_DAY_FABLE, 0.0),
                (TIER_SEVEN_DAY_OPUS, 6.0),
                (TIER_SEVEN_DAY_SONNET, 5.0)
            ]
        );
        assert_eq!(quota.tiers[0].resets_at, None);
    }

    #[test]
    fn claude_quota_skips_invalid_or_unrelated_scoped_rows() {
        let valid = scoped_limit("Fable", 37.0);
        let mut invalid = vec![serde_json::Value::Null, serde_json::json!("invalid")];
        for (pointer, value) in [
            ("/kind", serde_json::json!("spend")),
            ("/group", serde_json::json!("daily")),
            ("/percent", serde_json::Value::Null),
            ("/percent", serde_json::json!("37")),
            ("/percent", serde_json::json!(-1)),
            ("/resets_at", serde_json::json!(123)),
            ("/scope/model/display_name", serde_json::Value::Null),
            ("/scope/model/display_name", serde_json::json!("Unknown")),
            ("/scope/surface", serde_json::json!("claude_code")),
        ] {
            let mut row = valid.clone();
            *row.pointer_mut(pointer).unwrap() = value;
            invalid.push(row);
        }
        let mut body = serde_json::json!({
            "five_hour": { "utilization": 12.0 },
            "seven_day_fable": { "utilization": 8.0 },
            "limits": invalid
        });
        let fallback = parse_claude_quota(&body);
        assert_eq!(fallback.tiers.len(), 2);
        assert_eq!(fallback.tiers[1].utilization, 8.0);
        body["limits"].as_array_mut().unwrap().push(valid);
        let quota = parse_claude_quota(&body);
        assert_eq!(quota.tiers.len(), 2);
        assert_eq!(quota.tiers[0].utilization, 12.0);
        assert_eq!(quota.tiers[1].utilization, 37.0);
    }

    #[test]
    fn claude_quota_does_not_invent_missing_fable_usage() {
        for limits in [
            serde_json::Value::Null,
            serde_json::json!([]),
            serde_json::json!({}),
        ] {
            let quota = parse_claude_quota(&serde_json::json!({
                "five_hour": { "utilization": 12.0 },
                "limits": limits
            }));
            assert_eq!(quota.tiers.len(), 1);
            assert_eq!(quota.tiers[0].name, TIER_FIVE_HOUR);
        }
        let quota =
            parse_claude_quota(&serde_json::json!({ "limits": [scoped_limit("Fable", 100.0)] }));
        assert_eq!(quota.tiers.len(), 1);
        assert_eq!(quota.tiers[0].name, TIER_SEVEN_DAY_FABLE);
        assert_eq!(quota.tiers[0].utilization, 100.0);
    }

    #[test]
    fn window_seconds_map_to_expected_tier_names() {
        // 官方特例窗口
        assert_eq!(window_seconds_to_tier_name(18000), TIER_FIVE_HOUR);
        assert_eq!(window_seconds_to_tier_name(604800), TIER_SEVEN_DAY);
        // Codex 免费方案的次要窗口是 30 天（30 * 24 * 3600 = 2_592_000 秒）。
        // 前端 TIER_I18N_KEYS 与 tray 月分组都需要认得 "30_day"，见 #3651。
        assert_eq!(window_seconds_to_tier_name(2_592_000), TIER_THIRTY_DAY);
        // 其他窗口按小时/天回退命名
        assert_eq!(window_seconds_to_tier_name(3600), "1_hour");
        assert_eq!(window_seconds_to_tier_name(86400), "1_day");
    }

    // ── Antigravity 额度（v1.1.3 新接入）────────────────────

    /// 零依赖 mock：按顺序应答 `count` 个请求（loadCodeAssist → retrieveUserQuota）。
    /// 手法与 `services/coding_plan.rs` 的 `spawn_once_server` 一致。
    fn spawn_cloudcode_server(responses: Vec<String>) -> (String, std::thread::JoinHandle<()>) {
        let (base_url, handle, _) = spawn_cloudcode_server_capturing(responses);
        (base_url, handle)
    }

    /// 同上，但额外记录收到的请求原文（断言 UA 等请求头用）。
    fn spawn_cloudcode_server_capturing(
        responses: Vec<String>,
    ) -> (
        String,
        std::thread::JoinHandle<()>,
        std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        use std::io::{Read, Write};
        use std::sync::{Arc, Mutex};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind local listener");
        let port = listener.local_addr().expect("local addr").port();
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_in_thread = Arc::clone(&captured);
        let handle = std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                captured_in_thread
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).to_string());
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("http://127.0.0.1:{port}"), handle, captured)
    }

    fn http_response(status_line: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// 用临时 mock 主机跑 antigravity 额度查询（`base_url` 可注入正是为此）。
    async fn query_antigravity_against(
        responses: Vec<String>,
    ) -> Result<SubscriptionQuota, String> {
        let (base_url, handle) = spawn_cloudcode_server(responses);
        let result = query_cloudcode_quota(
            &CloudCodeTarget {
                tool: "antigravity",
                base_url: &base_url,
                ide_type: "ANTIGRAVITY",
                plugin_type: "CLOUD_CODE",
                relogin_hint: "agy",
                fallback_plan_name: Some("Antigravity"),
                user_agent: Some(ANTIGRAVITY_CLOUDCODE_USER_AGENT),
                family_tiers: true,
            },
            "fake-access-token",
        )
        .await;
        let _ = handle.join();
        result
    }

    #[tokio::test]
    async fn antigravity_quota_maps_cloudcode_buckets_to_model_tiers() {
        // Antigravity 配额聚合为两大模型族（对齐其官方 UI 分组）：
        // 同族取最低 remainingFraction（最受限窗口），族间 Gemini 在前。
        let load = http_response(
            "200 OK",
            r#"{"cloudaicompanionProject":"projects/antigravity-demo"}"#,
        );
        let quota = http_response(
            "200 OK",
            r#"{"buckets":[
                {"modelId":"gemini-3.8-flash","remainingFraction":0.25,"resetTime":"2026-09-25T00:00:00Z"},
                {"modelId":"gemini-3.8-pro","remainingFraction":0.75,"resetTime":"2026-09-25T00:00:00Z"},
                {"modelId":"claude-sonnet-4-6","remainingFraction":0.5,"resetTime":"2026-10-01T00:00:00Z"}
            ]}"#,
        );

        let result = query_antigravity_against(vec![load, quota])
            .await
            .expect("mock server should answer");

        assert!(result.success, "{:?}", result.error);
        assert_eq!(result.tool, "antigravity");
        assert_eq!(
            result
                .tiers
                .iter()
                .map(|t| (t.name.as_str(), t.utilization))
                .collect::<Vec<_>>(),
            // utilization = 已用百分比：gemini 族 min(0.25, 0.75) → 75%，
            // claude/gpt 族 0.5 → 50%
            vec![(TIER_GEMINI_FAMILY, 75.0), (TIER_CLAUDE_GPT_FAMILY, 50.0)]
        );
        assert_eq!(
            result.tiers[0].resets_at.as_deref(),
            Some("2026-09-25T00:00:00Z")
        );
        assert_eq!(
            result.tiers[1].resets_at.as_deref(),
            Some("2026-10-01T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn antigravity_quota_reports_expired_on_auth_failure() {
        let result = query_antigravity_against(vec![http_response("401 Unauthorized", "{}")])
            .await
            .expect("auth failure is an Ok(quota) with status, not a transport error");

        assert!(!result.success);
        assert_eq!(result.credential_status, CredentialStatus::Expired);
        let error = result.error.unwrap_or_default();
        assert!(error.contains("401"), "{error}");
        // 提示必须点名用户该去登录哪个 CLI
        assert!(error.contains("agy"), "{error}");
    }

    #[tokio::test]
    async fn antigravity_quota_surfaces_transport_errors_as_err() {
        // 端口上没有服务 → 连接失败：必须走 Err（前端 reject → retry + 保留上次成功值），
        // 不能折叠成「凭据过期」这类确定性状态。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let err = query_cloudcode_quota(
            &CloudCodeTarget {
                tool: "antigravity",
                base_url: &format!("http://127.0.0.1:{port}"),
                ide_type: "ANTIGRAVITY",
                plugin_type: "CLOUD_CODE",
                relogin_hint: "agy",
                fallback_plan_name: Some("Antigravity"),
                user_agent: Some(ANTIGRAVITY_CLOUDCODE_USER_AGENT),
                family_tiers: true,
            },
            "fake-access-token",
        )
        .await
        .expect_err("connection refused must be Err");
        assert!(err.contains("loadCodeAssist"), "{err}");
    }

    /// 隔离的临时 HOME（`antigravity_config` 的路径解析读 `CC_SWITCH_TEST_HOME`）
    fn with_test_home<T>(test_fn: impl FnOnce(&std::path::Path) -> T) -> T {
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

    fn write_agy_token_file(home: &std::path::Path, expiry: &str) {
        let dir = home.join(".gemini").join("antigravity-cli");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("antigravity-oauth-token"),
            serde_json::json!({
                "access_token": "agy-file-token",
                "token_type": "Bearer",
                "refresh_token": "agy-refresh",
                "expiry": expiry,
                "email": "user@example.com",
            })
            .to_string(),
        )
        .unwrap();
    }

    /// 造一枚与 agy 1.2.9 **实测形态**一致的 keyring 快照：
    /// `{"token": {access_token, token_type, refresh_token, expiry}, "auth_method", "id_token"}`
    /// —— `token` 是**嵌套对象**。曾经用 `"agy-live-token"` 这种字符串假数据做测试，
    /// 正好掩盖了"把 token 当字符串解析"的真实 bug（v1.1.3）。
    fn agy_keyring_snapshot(
        access_token: &str,
        expiry: &str,
    ) -> crate::antigravity_config::CredentialSnapshot {
        use base64::Engine as _;

        let blob = serde_json::json!({
            "token": {
                "access_token": access_token,
                "token_type": "Bearer",
                "refresh_token": "agy-refresh",
                "expiry": expiry,
            },
            "auth_method": "consumer",
            "id_token": "eyJhbGciOiJub25lIn0.fake.id-token",
        });
        crate::antigravity_config::CredentialSnapshot {
            target_name: "gemini:antigravity".to_string(),
            user_name: "antigravity".to_string(),
            blob: base64::engine::general_purpose::STANDARD.encode(blob.to_string()),
            persist: 2,
            cred_type: 1,
        }
    }

    fn keyring_with(
        snapshots: Vec<crate::antigravity_config::CredentialSnapshot>,
    ) -> impl Fn() -> Result<Vec<crate::antigravity_config::CredentialSnapshot>, crate::error::AppError>
    {
        move || Ok(snapshots.clone())
    }

    /// **回归主用例**：agy 把新 token 写在 keyring 里（嵌套对象形态），而 token 文件停在
    /// 首次登录那天（已过期）。旧实现按字符串读 `token` → 解析失败 → 回落到过期文件 →
    /// 误报「会话过期」。
    #[test]
    #[serial_test::serial]
    fn antigravity_credentials_read_nested_keyring_token_over_stale_file() {
        with_test_home(|home| {
            write_agy_token_file(home, "2020-01-01T00:00:00+00:00");

            let (token, status, message) =
                read_antigravity_credentials_with(keyring_with(vec![agy_keyring_snapshot(
                    "agy-keyring-token",
                    "2999-01-01T00:00:00+00:00",
                )]));

            assert_eq!(token.as_deref(), Some("agy-keyring-token"));
            assert_eq!(status, CredentialStatus::Valid);
            assert!(message.is_none());
        });
    }

    #[test]
    #[serial_test::serial]
    fn antigravity_credentials_accept_legacy_string_shaped_keyring_blob() {
        with_test_home(|_home| {
            use base64::Engine as _;

            let blob = base64::engine::general_purpose::STANDARD
                .encode(serde_json::json!({ "token": "agy-legacy-token" }).to_string());
            let snapshot = crate::antigravity_config::CredentialSnapshot {
                target_name: "gemini:antigravity".to_string(),
                user_name: "antigravity".to_string(),
                blob,
                persist: 2,
                cred_type: 1,
            };

            let (token, status, _) =
                read_antigravity_credentials_with(keyring_with(vec![snapshot]));

            assert_eq!(token.as_deref(), Some("agy-legacy-token"));
            assert_eq!(status, CredentialStatus::Valid);
        });
    }

    #[test]
    #[serial_test::serial]
    fn antigravity_credentials_read_fresh_token_file() {
        with_test_home(|home| {
            write_agy_token_file(home, "2999-01-01T00:00:00+00:00");

            let (token, status, message) = read_antigravity_credentials_with(keyring_with(vec![]));

            assert_eq!(token.as_deref(), Some("agy-file-token"));
            assert_eq!(status, CredentialStatus::Valid);
            assert!(message.is_none());
        });
    }

    #[test]
    #[serial_test::serial]
    fn antigravity_credentials_pick_the_freshest_of_the_two_sources() {
        with_test_home(|home| {
            // 反向：keyring 里是旧 token，文件反而是新的 → 用文件的
            write_agy_token_file(home, "2999-01-01T00:00:00+00:00");

            let (token, status, _) =
                read_antigravity_credentials_with(keyring_with(vec![agy_keyring_snapshot(
                    "agy-keyring-stale",
                    "2020-01-01T00:00:00+00:00",
                )]));

            assert_eq!(token.as_deref(), Some("agy-file-token"));
            assert_eq!(status, CredentialStatus::Valid);
        });
    }

    #[test]
    #[serial_test::serial]
    fn antigravity_credentials_report_expired_when_everything_is_stale() {
        with_test_home(|home| {
            write_agy_token_file(home, "2020-01-01T00:00:00+00:00");

            let (token, status, message) =
                read_antigravity_credentials_with(keyring_with(vec![agy_keyring_snapshot(
                    "agy-keyring-stale",
                    "2020-06-01T00:00:00+00:00",
                )]));

            // 仍把最近的一枚交回去（调用方会试一把），但状态是"已过期"
            assert_eq!(token.as_deref(), Some("agy-keyring-stale"));
            assert_eq!(status, CredentialStatus::Expired);
            assert!(message.unwrap_or_default().contains("expired"));
        });
    }

    #[test]
    #[serial_test::serial]
    fn antigravity_credentials_report_not_found_and_parse_error() {
        with_test_home(|home| {
            // 既无 keyring 条目也无 token 文件 → not_found（前端显示「请先运行 agy 登录」）
            let (token, status, _) = read_antigravity_credentials_with(keyring_with(vec![]));
            assert!(token.is_none());
            assert_eq!(status, CredentialStatus::NotFound);

            // 有文件但内容损坏 → parse_error（而不是静默消失）
            let dir = home.join(".gemini").join("antigravity-cli");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("antigravity-oauth-token"), "{not json").unwrap();
            let (_, status, message) = read_antigravity_credentials_with(keyring_with(vec![]));
            assert_eq!(status, CredentialStatus::ParseError);
            assert!(message.is_some());

            // keyring 条目存在但里面没有 access_token → 也算 parse_error，而不是假装没登录
            use base64::Engine as _;
            let empty_blob = base64::engine::general_purpose::STANDARD
                .encode(serde_json::json!({ "token": { "token_type": "Bearer" } }).to_string());
            let broken = crate::antigravity_config::CredentialSnapshot {
                target_name: "gemini:antigravity".to_string(),
                user_name: "antigravity".to_string(),
                blob: empty_blob,
                persist: 2,
                cred_type: 1,
            };
            std::fs::remove_file(dir.join("antigravity-oauth-token")).unwrap();
            let (_, status, message) =
                read_antigravity_credentials_with(keyring_with(vec![broken]));
            assert_eq!(status, CredentialStatus::ParseError);
            assert!(message.is_some());
        });
    }

    #[test]
    fn cloudcode_tier_fallback_prefers_name_then_falls_back_to_id() {
        // 实测形态：name 存在（Antigravity 免费档）
        let load = serde_json::json!({
            "currentTier": { "id": "free-tier", "name": "Antigravity", "description": "…" }
        });
        let tier = cloudcode_tier_fallback(&load).expect("tier");
        assert_eq!(tier.name, format!("{TIER_CURRENT_PLAN_PREFIX}Antigravity"));
        assert_eq!(tier.utilization, 0.0);
        assert!(tier.resets_at.is_none());

        // 上游形态变化：只有 id 没有 name（如 {"id":"free-tier"}）→ 用 id 兜底
        let load = serde_json::json!({ "currentTier": { "id": "free-tier" } });
        let tier = cloudcode_tier_fallback(&load).expect("tier from id");
        assert_eq!(tier.name, format!("{TIER_CURRENT_PLAN_PREFIX}free-tier"));

        // 两者都无 → None（交给 403 分支的工具名兜底）
        assert!(cloudcode_tier_fallback(&serde_json::json!({
            "currentTier": {}
        }))
        .is_none());
        assert!(cloudcode_tier_fallback(&serde_json::json!({})).is_none());
    }

    #[tokio::test]
    async fn antigravity_quota_403_subscription_required_falls_back_to_plan_tier() {
        // v1.1.3 实测：免费档 retrieveUserQuota 403 + SUBSCRIPTION_REQUIRED，
        // 账号登录正常（loadCodeAssist 200）。必须显示套餐 tier，
        // 而不是把裸 403 塞给用户（2026-09-24 用户截图）。
        let load = http_response(
            "200 OK",
            r#"{"currentTier":{"id":"free-tier","name":"Antigravity"}}"#,
        );
        let quota_403 = http_response(
            "403 Forbidden",
            r#"{"error":{"code":403,"message":"You do not have a valid license of this product.","status":"PERMISSION_DENIED","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"SUBSCRIPTION_REQUIRED"}]}}"#,
        );

        let result = query_antigravity_against(vec![load, quota_403])
            .await
            .expect("403 is an Ok(quota) with fallback, not a transport error");

        assert!(
            result.success,
            "fallback must be success: {:?}",
            result.error
        );
        assert_eq!(
            result.tiers[0].name,
            format!("{TIER_CURRENT_PLAN_PREFIX}Antigravity")
        );
    }

    #[tokio::test]
    async fn antigravity_quota_403_without_current_tier_still_falls_back() {
        // 兜底条件太窄曾让用户看到一屏裸 403：即使 loadCodeAssist 没有
        // currentTier，SUBSCRIPTION_REQUIRED 也要给出「套餐 <工具名>」。
        let load = http_response("200 OK", r#"{}"#);
        let quota_403 = http_response(
            "403 Forbidden",
            r#"{"error":{"code":403,"message":"no license","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"SUBSCRIPTION_REQUIRED"}]}}"#,
        );

        let result = query_antigravity_against(vec![load, quota_403])
            .await
            .expect("403 fallback is Ok");

        assert!(result.success);
        assert_eq!(
            result.tiers[0].name,
            format!("{TIER_CURRENT_PLAN_PREFIX}Antigravity")
        );
    }

    #[test]
    fn antigravity_client_metadata_matches_proto_enum_members() {
        // ideType/pluginType 必须是 Cloud Code ClientMetadata 的枚举成员名
        // （枚举表取自 agy 内嵌的 proto 描述符）；写错了服务端会拒绝整条请求。
        let target = CloudCodeTarget {
            tool: "antigravity",
            base_url: ANTIGRAVITY_CLOUDCODE_BASE,
            ide_type: "ANTIGRAVITY",
            plugin_type: "CLOUD_CODE",
            relogin_hint: "agy",
            fallback_plan_name: Some("Antigravity"),
            user_agent: Some(ANTIGRAVITY_CLOUDCODE_USER_AGENT),
            family_tiers: true,
        };
        assert!(ANTIGRAVITY_CLOUDCODE_BASE.starts_with("https://"));
        assert_eq!(target.ide_type, "ANTIGRAVITY");
        // 前端提示里的 CLI 名（SharedCLIName）不该写成 appId
        assert_eq!(target.relogin_hint, "agy");
        // Cloud Code 上游按 UA 过滤客户端（实测无 Antigravity UA 时
        // fetchAvailableModels 403），antigravity 目标必须带 UA。
        assert_eq!(
            target.user_agent,
            Some("vscode/1.100.0 (Antigravity/4.3.0)")
        );
    }

    #[tokio::test]
    async fn antigravity_quota_403_falls_back_to_fetch_available_models() {
        // retrieveUserQuota 被许可墙挡住（403 SUBSCRIPTION_REQUIRED，UA 无关）时，
        // fetchAvailableModels（带 Antigravity UA 可用）承担真实配额数据源：
        // 聚合规则与 buckets 路径一致（两族归类、每族取最低 remaining）。
        let load = http_response(
            "200 OK",
            r#"{"cloudaicompanionProject":"projects/antigravity-demo"}"#,
        );
        let quota_403 = http_response(
            "403 Forbidden",
            r#"{"error":{"code":403,"message":"no license","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"SUBSCRIPTION_REQUIRED"}]}}"#,
        );
        let models = http_response(
            "200 OK",
            r#"{"models":{
                "chat_20706":{"isInternal":true,"quotaInfo":{"remainingFraction":0.1}},
                "chat_23310":{"quotaInfo":{"remainingFraction":0.2}},
                "gemini-3.5-flash-high":{"displayName":"Gemini 3.5 Flash (High)","quotaInfo":{"remainingFraction":0.75,"resetTime":"2026-10-01T17:13:59Z"}},
                "gemini-3.5-flash-low":{"quotaInfo":{"remainingFraction":0.9}},
                "gemini-3-pro":{"quotaInfo":{"remainingFraction":0.5,"resetTime":"2026-10-01T00:00:00Z"}},
                "claude-sonnet-4-6":{"quotaInfo":{"remainingFraction":0.2,"resetTime":"2026-10-01T18:00:00Z"}},
                "claude-opus-4-6-thinking":{"quotaInfo":{"remainingFraction":0.4}},
                "gpt-oss-120b-medium":{"quotaInfo":{"remainingFraction":0.6}},
                "no-quota-model":{"displayName":"NoQuota"}
            }}"#,
        );

        let result = query_antigravity_against(vec![load, quota_403, models])
            .await
            .expect("fetchAvailableModels fallback is Ok");

        assert!(result.success);
        let names: Vec<&str> = result.tiers.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec![TIER_GEMINI_FAMILY, TIER_CLAUDE_GPT_FAMILY]);
        // 每族取最低 remaining：gemini 族 min(0.75, 0.9, 0.5) = 0.5 → 已用 50%；
        // claude/gpt 族 min(0.2, 0.4, 0.6) = 0.2 → 已用 80%
        assert_eq!(result.tiers[0].utilization, 50.0);
        assert_eq!(result.tiers[1].utilization, 80.0);
        assert_eq!(
            result.tiers[0].resets_at.as_deref(),
            Some("2026-10-01T00:00:00Z")
        );
        assert_eq!(
            result.tiers[1].resets_at.as_deref(),
            Some("2026-10-01T18:00:00Z")
        );
    }

    #[tokio::test]
    async fn antigravity_quota_sends_antigravity_user_agent() {
        // Cloud Code 上游按 UA 过滤客户端：无 Antigravity UA 时 fetchAvailableModels
        // 403（2026-09-24 实测）。UA 一旦丢失，配额会静默退回套餐兜底——这里锁死。
        let load = http_response("200 OK", r#"{}"#);
        let quota_403 = http_response("403 Forbidden", r#"{"error":{"message":"no license"}}"#);
        let models = http_response(
            "200 OK",
            r#"{"models":{"gemini-3.5-flash":{"quotaInfo":{"remainingFraction":0.9}}}}"#,
        );

        let (base_url, handle, captured) =
            spawn_cloudcode_server_capturing(vec![load, quota_403, models]);
        let result = query_cloudcode_quota(
            &CloudCodeTarget {
                tool: "antigravity",
                base_url: &base_url,
                ide_type: "ANTIGRAVITY",
                plugin_type: "CLOUD_CODE",
                relogin_hint: "agy",
                fallback_plan_name: Some("Antigravity"),
                user_agent: Some(ANTIGRAVITY_CLOUDCODE_USER_AGENT),
                family_tiers: true,
            },
            "fake-access-token",
        )
        .await;
        let _ = handle.join();

        let requests = captured.lock().unwrap();
        let models_request = requests
            .iter()
            .find(|r| r.contains("v1internal:fetchAvailableModels"))
            .expect("fetchAvailableModels request must be captured");
        assert!(
            models_request
                .to_lowercase()
                .contains("user-agent: vscode/1.100.0 (antigravity/4.3.0)"),
            "UA header missing on fetchAvailableModels request: {models_request}"
        );
        assert!(result.expect("models fallback is Ok").success);
    }

    #[tokio::test]
    async fn antigravity_quota_403_with_models_unavailable_still_falls_back_to_plan_tier() {
        // fetchAvailableModels 也失败（HTTP 500）→ 落回 loadCodeAssist currentTier 套餐兜底。
        let load = http_response("200 OK", r#"{}"#);
        let quota_403 = http_response(
            "403 Forbidden",
            r#"{"error":{"code":403,"message":"no license","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"SUBSCRIPTION_REQUIRED"}]}}"#,
        );
        let models_500 = http_response(
            "500 Internal Server Error",
            r#"{"error":{"message":"boom"}}"#,
        );

        let result = query_antigravity_against(vec![load, quota_403, models_500])
            .await
            .expect("plan-tier fallback is Ok");
        assert!(result.success);
        assert_eq!(
            result.tiers[0].name,
            format!("{TIER_CURRENT_PLAN_PREFIX}Antigravity")
        );
    }

    // 探针结论详见 `try_query_available_models_quota` 与
    // `ANTIGRAVITY_CLOUDCODE_USER_AGENT` 的文档注释（探针/live-check 测试已删除）。
}
