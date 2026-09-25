//! Grok (xAI) 官方订阅额度查询
//!
//! 读取 Grok CLI 的 OAuth 凭据（~/.grok/auth.json），调用 Grok 的 CLI 代理
//! billing 端点查询订阅用量。
//!
//! 对齐 CodexBar（steipete/CodexBar）现行版本的 Grok provider：
//! - 凭据：auth.json 是以 OIDC scope URL 为 key 的 map，优先
//!   `https://auth.x.ai::<client-id>` 条目；`key` 字段即 Bearer token。
//! - 查询：GET `cli-chat-proxy.grok.com/v1/billing?format=credits` —— 结构化
//!   JSON（creditUsagePercent / currentPeriod / onDemand*）。2026-09-25 实测：
//!   免费计划只回周期起止、无百分比 → 卡片显示「用量未知」而不是伪造的 0%
//!   （旧版 gRPC 启发式扫描因此恒输出 0%）。
//! - token 刷新由 Grok CLI 自己负责（6 小时短命 token，CLI 自动续期），
//!   本模块只读不刷新，过期时引导用户重新 `grok login`。

use std::time::{SystemTime, UNIX_EPOCH};

use crate::services::subscription::{
    CredentialStatus, QuotaTier, SubscriptionQuota, TIER_CREDITS, TIER_MONTHLY, TIER_WEEKLY_LIMIT,
};

const GROK_BILLING_ENDPOINT: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";

/// SuperGrok（OIDC）条目的 scope 前缀
const OIDC_SCOPE_PREFIX: &str = "https://auth.x.ai::";
/// 旧版 `grok login` 的 session scope
const LEGACY_SESSION_SCOPE: &str = "https://accounts.x.ai/sign-in";

const RELOGIN_HINT: &str = "Please re-login with `grok login`.";

// ── 凭据读取 ──────────────────────────────────────────────

/// (access_token, status, message)
type GrokCredentials = (Option<String>, CredentialStatus, Option<String>);

/// 读取 Grok CLI 的 OAuth 凭据（~/.grok/auth.json，目录可被设置覆盖）
fn read_grok_credentials() -> GrokCredentials {
    let auth_path = crate::grok_config::get_grok_config_dir().join("auth.json");

    if !auth_path.exists() {
        return (None, CredentialStatus::NotFound, None);
    }

    let content = match std::fs::read_to_string(&auth_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to read Grok auth file: {e}")),
            );
        }
    };

    parse_grok_auth_json(&content)
}

/// 解析 auth.json：顶层是 scope → 条目的 map，选出首选条目并检查过期
fn parse_grok_auth_json(content: &str) -> GrokCredentials {
    let parsed: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(e) => {
            return (
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse Grok auth JSON: {e}")),
            );
        }
    };

    let root = match parsed.as_object() {
        Some(o) => o,
        None => {
            return (
                None,
                CredentialStatus::ParseError,
                Some("Grok auth.json root is not an object".to_string()),
            );
        }
    };

    let entry = match select_preferred_entry(root) {
        Some(e) => e,
        None => {
            return (
                None,
                CredentialStatus::ParseError,
                Some("Grok auth.json contains no usable access token".to_string()),
            );
        }
    };

    // select_preferred_entry 已保证 key 非空
    let access_token = entry
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    if let Some(expires_at) = entry.get("expires_at").and_then(|v| v.as_str()) {
        if is_iso_expired(expires_at) {
            return (
                Some(access_token),
                CredentialStatus::Expired,
                Some("Grok OAuth token has expired".to_string()),
            );
        }
    }

    (Some(access_token), CredentialStatus::Valid, None)
}

/// 选择首选凭据条目：OIDC（SuperGrok）优先，legacy session 兜底。
///
/// 只接受 `key` 非空的条目——残缺的 OIDC 记录不能遮蔽健康的 legacy 条目
/// （与 CodexBar `selectPreferredEntry` 一致）。
fn select_preferred_entry(
    root: &serde_json::Map<String, serde_json::Value>,
) -> Option<&serde_json::Map<String, serde_json::Value>> {
    let mut oidc_candidate = None;
    let mut legacy_candidate = None;

    for (scope, value) in root {
        let entry = match value.as_object() {
            Some(e) => e,
            None => continue,
        };
        let has_key = entry
            .get("key")
            .and_then(|v| v.as_str())
            .is_some_and(|k| !k.is_empty());
        if !has_key {
            continue;
        }
        if scope.starts_with(OIDC_SCOPE_PREFIX) {
            oidc_candidate = Some(entry);
        } else if scope == LEGACY_SESSION_SCOPE || scope.contains("/sign-in") {
            legacy_candidate = Some(entry);
        }
    }

    oidc_candidate.or(legacy_candidate)
}

/// 判断 ISO 8601 时间串是否已过期；无法解析时不视为过期
fn is_iso_expired(iso: &str) -> bool {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso) {
        dt.timestamp() < now_secs
    } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S%.f") {
        dt.and_utc().timestamp() < now_secs
    } else {
        false
    }
}

/// billing 快照（JSON 端点）
#[derive(Debug)]
struct GrokBillingSnapshot {
    /// 已用百分比；上游不报告（免费计划只有周期起止）时为 None → 前端显示「用量未知」
    used_percent: Option<f64>,
    /// 重置时间（ISO 8601，原样透传）
    resets_at: Option<String>,
    /// 周期类型（如 USAGE_PERIOD_TYPE_WEEKLY）
    period_type: Option<String>,
}

fn parse_billing_payload(data: &[u8]) -> Result<GrokBillingSnapshot, String> {
    let body: serde_json::Value =
        serde_json::from_slice(data).map_err(|e| format!("response is not valid JSON: {e}"))?;
    let config = body
        .get("config")
        .ok_or_else(|| "response has no config object".to_string())?;

    let percent = config
        .get("creditUsagePercent")
        .and_then(serde_json::Value::as_f64)
        .filter(|p| p.is_finite());

    let resets_at = config
        .get("currentPeriod")
        .and_then(|p| p.get("end"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            config
                .get("billingPeriodEnd")
                .and_then(serde_json::Value::as_str)
        })
        .map(str::to_string);

    let period_type = config
        .get("currentPeriod")
        .and_then(|p| p.get("type"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    // on-demand 兜底：cap > 0 时用 used/cap
    let on_demand_percent = (|| {
        let cap = config
            .get("onDemandCap")
            .and_then(|v| v.get("val"))
            .and_then(serde_json::Value::as_f64)?;
        let used = config
            .get("onDemandUsed")
            .and_then(|v| v.get("val"))
            .and_then(serde_json::Value::as_f64)?;
        if cap <= 0.0 {
            return None;
        }
        Some(((used / cap) * 100.0).clamp(0.0, 100.0))
    })();

    if percent.is_none() && on_demand_percent.is_none() && resets_at.is_none() {
        return Err("response has neither a usage percent nor a billing period".to_string());
    }

    Ok(GrokBillingSnapshot {
        used_percent: percent.or(on_demand_percent),
        resets_at,
        period_type,
    })
}

/// tier 归类：优先周期类型（WEEKLY / MONTHLY），否则按重置距离兜底
fn tier_name_for_period(snapshot: &GrokBillingSnapshot, now_secs: i64) -> String {
    if let Some(pt) = &snapshot.period_type {
        if pt.contains("WEEKLY") {
            return TIER_WEEKLY_LIMIT.to_string();
        }
        if pt.contains("MONTHLY") {
            return TIER_MONTHLY.to_string();
        }
    }
    let resets_secs = snapshot
        .resets_at
        .as_deref()
        .and_then(|iso| chrono::DateTime::parse_from_rfc3339(iso).ok())
        .map(|dt| dt.timestamp());
    tier_name_for_reset(resets_secs, now_secs).to_string()
}

fn tier_name_for_reset(resets_at: Option<i64>, now_secs: i64) -> &'static str {
    if let Some(ts) = resets_at {
        let days = ((ts - now_secs) as f64 / 86400.0).round() as i64;
        if (4..=12).contains(&days) {
            return TIER_WEEKLY_LIMIT;
        }
        if (20..=45).contains(&days) {
            return TIER_MONTHLY;
        }
    }
    TIER_CREDITS
}

/// 查询 Grok 官方订阅额度
///
/// 与 claude/codex/gemini 同一约定：瞬时传输失败返回 `Err`（前端 retry +
/// 保留上次成功值），确定性失败返回 `Ok(success:false)`。
///
/// 参数化 `tool_label` / `relogin_hint` 让该函数可被两个调用点共用（与
/// `query_codex_quota` 的双调用点设计一致）：
/// - `"grokbuild"` + "grok login"（Grok CLI 凭据路径）
/// - `"xai_oauth"` + "re-login via OGG Switch"（OGG Switch 自管 xAI OAuth 路径，
///   见 `commands::xai_oauth::get_xai_oauth_quota`；两者是同一个 OAuth client，
///   token 对 grok.com 账单端点等效）
pub(crate) async fn query_grok_quota(
    access_token: &str,
    tool_label: &str,
    relogin_hint: &str,
) -> Result<SubscriptionQuota, String> {
    let client = crate::proxy::http_client::get();
    let resp = client
        .get(GROK_BILLING_ENDPOINT)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(SubscriptionQuota::error(
            tool_label,
            CredentialStatus::Expired,
            format!("Authentication failed (HTTP {status}). {relogin_hint}"),
        ));
    }

    let raw = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response: {e}"))?;

    if !status.is_success() {
        // 408 / 5xx 视为瞬时（前端 retry + keep-last-good），其余按确定性失败透出
        if status.as_u16() == 408 || status.is_server_error() {
            return Err(format!("HTTP {status}"));
        }
        return Ok(SubscriptionQuota::error(
            tool_label,
            CredentialStatus::Valid,
            format!(
                "HTTP {status}: {}",
                String::from_utf8_lossy(&raw)
                    .chars()
                    .take(200)
                    .collect::<String>()
            ),
        ));
    }

    let snapshot =
        parse_billing_payload(&raw).map_err(|e| format!("Failed to parse API response: {e}"))?;

    let tier = QuotaTier {
        name: tier_name_for_period(&snapshot, now_secs()),
        utilization: snapshot.used_percent.unwrap_or(0.0),
        resets_at: snapshot.resets_at.clone(),
        used_value_usd: None,
        max_value_usd: None,
        utilization_unknown: Some(snapshot.used_percent.is_none()),
    };

    Ok(SubscriptionQuota {
        tool: tool_label.to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers: vec![tier],
        extra_usage: None,
        error: None,
        queried_at: Some(now_millis()),
    })
}

/// grokbuild 的订阅额度入口（由 `subscription::get_subscription_quota` 分发）
pub(crate) async fn get_grok_subscription_quota() -> Result<SubscriptionQuota, String> {
    let (token, status, message) = read_grok_credentials();

    match status {
        CredentialStatus::NotFound => Ok(SubscriptionQuota::not_found("grokbuild")),
        CredentialStatus::ParseError => Ok(SubscriptionQuota::error(
            "grokbuild",
            CredentialStatus::ParseError,
            message.unwrap_or_else(|| "Failed to parse Grok credentials".to_string()),
        )),
        CredentialStatus::Expired => {
            // 即使过期也尝试调用 API（时钟偏差时 token 可能仍有效）
            if let Some(ref token) = token {
                let result = query_grok_quota(token, "grokbuild", RELOGIN_HINT).await?;
                if result.success {
                    return Ok(result);
                }
            }
            Ok(SubscriptionQuota::error(
                "grokbuild",
                CredentialStatus::Expired,
                format!(
                    "{} {RELOGIN_HINT}",
                    message.unwrap_or_else(|| "Grok OAuth token has expired.".to_string())
                ),
            ))
        }
        CredentialStatus::Valid => {
            let token = token.expect("token must be Some when status is Valid");
            query_grok_quota(&token, "grokbuild", RELOGIN_HINT).await
        }
    }
}

// ── 辅助函数 ──────────────────────────────────────────────

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实响应（2026-09-25 抓包，免费计划：无百分比、只有周期起止）
    const REAL_WEEKLY_NO_PERCENT: &str = r#"{"config":{"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","start":"2026-09-19T06:13:58.784818+00:00","end":"2026-09-26T06:13:58.784818+00:00"},"onDemandCap":{"val":0},"onDemandUsed":{"val":0},"isUnifiedBillingUser":true,"prepaidBalance":{"val":0},"topUpMethod":"TOP_UP_METHOD_SAVED_PAYMENT_METHOD","billingPeriodStart":"2026-09-19T06:13:58.784818+00:00","billingPeriodEnd":"2026-09-26T06:13:58.784818+00:00"}}"#;

    #[test]
    fn weekly_period_without_percent_marks_utilization_unknown() {
        let snapshot = parse_billing_payload(REAL_WEEKLY_NO_PERCENT.as_bytes()).unwrap();
        assert_eq!(snapshot.used_percent, None);
        assert_eq!(
            snapshot.period_type.as_deref(),
            Some("USAGE_PERIOD_TYPE_WEEKLY")
        );
        assert_eq!(
            snapshot.resets_at.as_deref(),
            Some("2026-09-26T06:13:58.784818+00:00")
        );
        assert_eq!(
            tier_name_for_period(&snapshot, now_secs()),
            TIER_WEEKLY_LIMIT
        );
    }

    #[test]
    fn credit_usage_percent_is_used_directly() {
        let body = r#"{"config":{"creditUsagePercent":37.5,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","end":"2026-09-26T00:00:00Z"}}}"#;
        let snapshot = parse_billing_payload(body.as_bytes()).unwrap();
        assert_eq!(snapshot.used_percent, Some(37.5));
    }

    #[test]
    fn on_demand_fallback_uses_used_over_cap() {
        let body = r#"{"config":{"onDemandCap":{"val":1000},"onDemandUsed":{"val":250},"billingPeriodEnd":"2026-10-01T00:00:00Z"}}"#;
        let snapshot = parse_billing_payload(body.as_bytes()).unwrap();
        assert_eq!(snapshot.used_percent, Some(25.0));
    }

    #[test]
    fn response_without_percent_or_period_is_an_error() {
        assert!(parse_billing_payload(r#"{"config":{}}"#.as_bytes()).is_err());
    }

    #[test]
    fn billing_endpoint_is_the_cli_proxy_json_api() {
        // gRPC 端点对 Bearer token 只回周期配置（无用量百分比）——CLI 代理 JSON
        // 端点才是 Bearer 路径的受支持入口（对齐 CodexBar GrokCreditsProxyFetcher）
        assert!(GROK_BILLING_ENDPOINT.starts_with("https://cli-chat-proxy.grok.com/"));
        assert!(GROK_BILLING_ENDPOINT.contains("format=credits"));
    }
}
