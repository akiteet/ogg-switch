//! Antigravity（agy）专用命令
//!
//! Google 账号是独立池，不是供应商。官方登录始终保持当前供应商 =
//! `antigravity-official`。导入把当前 agy 登录快照进账号池（按 refresh_token
//! 指纹去重）；切换账号只恢复 token / Windows 凭据。

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::app_config::AppType;
use crate::database::AntigravityAccount;
use crate::error::AppError;
use crate::provider::Provider;
use crate::store::AppState;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;

/// 导入结果（前端按 outcome 展示提示）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityImportOutcome {
    /// imported-api-key | imported-account | account-updated | skipped | not-found
    pub outcome: String,
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// 找不到凭据时的探测明细
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<String>,
}

const ACCOUNT_ID_PREFIX: &str = "antigravity-account-";
/// 指纹取 SHA-256 前 6 字节的十六进制（12 字符），足够防碰撞且 id 可读
const FINGERPRINT_BYTES: usize = 6;

fn outcome(kind: &str, provider_id: Option<String>) -> AntigravityImportOutcome {
    AntigravityImportOutcome {
        outcome: kind.to_string(),
        provider_id,
        account_id: None,
        diagnostics: None,
    }
}

fn account_outcome(kind: &str, account_id: String) -> AntigravityImportOutcome {
    AntigravityImportOutcome {
        outcome: kind.to_string(),
        provider_id: Some(crate::database::ANTIGRAVITY_OFFICIAL_PROVIDER_ID.to_string()),
        account_id: Some(account_id),
        diagnostics: None,
    }
}

fn ensure_official_current(state: &AppState) -> Result<(), AppError> {
    let app = AppType::Antigravity;
    let app_str = app.as_str();
    state.db.ensure_official_seed_by_id(
        crate::database::ANTIGRAVITY_OFFICIAL_PROVIDER_ID,
        app.clone(),
    )?;
    state
        .db
        .set_current_provider(app_str, crate::database::ANTIGRAVITY_OFFICIAL_PROVIDER_ID)?;
    crate::settings::set_current_provider(
        &app,
        Some(crate::database::ANTIGRAVITY_OFFICIAL_PROVIDER_ID),
    )?;
    Ok(())
}

/// 从登录凭据载荷（{"token":…} 文件快照 / {"credential":…} Windows 凭据快照）
/// 提取指纹材料与显示信息。凭据 blob 通常就是 token JSON（agy 各平台同构）。
fn credential_material(auth_payload: &Value) -> (String, Option<String>) {
    if let Some(token) = auth_payload.get("token") {
        let source = token.get("token").unwrap_or(token);
        let material = first_token_string(source)
            .or_else(|| first_token_string(token))
            .unwrap_or_else(|| token.to_string());
        let email = find_email(token).or_else(|| find_email(source));
        return (material, email);
    }
    if let Some(credential) = auth_payload.get("credential") {
        if let Some(parsed) = decode_credential_blob(credential) {
            let nested = parsed.get("token").unwrap_or(&parsed);
            let material = first_token_string(nested)
                .or_else(|| first_token_string(&parsed))
                .unwrap_or_else(|| parsed.to_string());
            let email = find_email(&parsed).or_else(|| find_email(nested));
            return (material, email);
        }
        let blob = credential
            .get("blob")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return (blob.to_string(), None);
    }
    (auth_payload.to_string(), find_email(auth_payload))
}

fn first_token_string(value: &Value) -> Option<String> {
    value
        .get("refresh_token")
        .or_else(|| value.get("access_token"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn find_email(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if s.contains('@') && !s.contains(' ') && s.len() < 254 => Some(s.clone()),
        Value::Object(map) => {
            for key in ["email", "user_email", "account"] {
                if let Some(found) = map
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|s| s.contains('@'))
                {
                    return Some(found.to_string());
                }
            }
            if let Some(id_token) = map.get("id_token").and_then(Value::as_str) {
                if let Some(email) = email_from_jwt(id_token) {
                    return Some(email);
                }
            }
            for nested in map.values() {
                if let Some(email) = find_email(nested) {
                    return Some(email);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(find_email),
        _ => None,
    }
}

fn email_from_jwt(token: &str) -> Option<String> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("email")
        .and_then(Value::as_str)
        .filter(|s| s.contains('@'))
        .map(str::to_string)
}

fn decode_credential_blob(credential: &Value) -> Option<Value> {
    let blob = credential.get("blob").and_then(Value::as_str)?;
    let bytes = BASE64_STANDARD.decode(blob).ok()?;
    if let Ok(text) = String::from_utf8(bytes.clone()) {
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            return Some(value);
        }
    }
    if bytes.len() >= 2 && bytes.len() % 2 == 0 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let text = String::from_utf16_lossy(&units);
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            return Some(value);
        }
    }
    None
}

#[tauri::command]
pub async fn import_antigravity_from_live(
    state: State<'_, AppState>,
) -> Result<AntigravityImportOutcome, String> {
    let mut preloaded = if crate::antigravity_config::is_api_key_live() {
        None
    } else {
        crate::antigravity_config::discover_local_auth().unwrap_or(None)
    };
    if let Some(payload) = preloaded.as_mut() {
        enrich_payload_email(payload).await;
    }
    import_from_live_with(state.inner(), preloaded).map_err(|e| e.to_string())
}

/// 已有凭据往往没有 email 字段；用 access_token 查一次 userinfo 补上显示名。
async fn enrich_payload_email(payload: &mut Value) {
    if find_email(payload).is_some() {
        return;
    }
    let Some(access_token) = payload
        .get("token")
        .and_then(|t| t.get("token").or(Some(t)))
        .and_then(|t| t.get("access_token"))
        .and_then(Value::as_str)
    else {
        return;
    };
    let Some(email) = crate::antigravity_config::fetch_email_for_access_token(access_token).await
    else {
        return;
    };
    let Some(token_obj) = payload.get_mut("token").and_then(Value::as_object_mut) else {
        return;
    };
    let nested_is_object = token_obj.get("token").is_some_and(Value::is_object);
    let target = if nested_is_object {
        token_obj.get_mut("token").and_then(Value::as_object_mut)
    } else {
        Some(token_obj)
    };
    if let Some(target) = target {
        target.insert("email".to_string(), Value::String(email));
    }
}

pub(crate) fn import_from_live_with(
    state: &AppState,
    preloaded: Option<Value>,
) -> Result<AntigravityImportOutcome, AppError> {
    let app = AppType::Antigravity;
    let app_str = app.as_str();

    // 1. API key / 中转站态：一次性 default 导入（对齐通用导入的守卫语义）
    if crate::antigravity_config::is_api_key_live() {
        if state.db.has_non_official_seed_provider(app_str)? {
            return Ok(outcome("skipped", None));
        }
        let settings_config = crate::antigravity_config::read_antigravity_live_settings()?;
        let mut provider = Provider::with_id(
            "default".to_string(),
            "default".to_string(),
            settings_config,
            None,
        );
        provider.category = Some("custom".to_string());
        state.db.save_provider(app_str, &provider)?;
        state.db.set_current_provider(app_str, "default")?;
        crate::settings::set_current_provider(&app, Some("default"))?;
        return Ok(outcome("imported-api-key", Some("default".to_string())));
    }

    // 2. Google 登录态：写入账号池，当前供应商保持 Official
    let discovered = match preloaded {
        Some(payload) => Some(payload),
        None => crate::antigravity_config::discover_local_auth()?,
    };
    if let Some(auth_payload) = discovered {
        return import_account(state, auth_payload);
    }

    Err(AppError::Message(diagnostics()))
}

/// official 态的探测明细：说明找过哪里、看到了什么，便于用户自查
fn diagnostics() -> String {
    let token_path = crate::antigravity_config::get_antigravity_token_path();
    let token_found = token_path.exists();
    let credential_hits = crate::antigravity_config::credential_manager::find_matching()
        .map(|list| {
            if list.is_empty() {
                "凭据管理器：未发现 agy/gemini/antigravity 相关条目".to_string()
            } else {
                format!("凭据管理器命中 {} 条（但均无凭据内容）", list.len())
            }
        })
        .unwrap_or_else(|e| format!("凭据管理器枚举失败: {e}"));
    format!(
        "已探测：登录凭据文件 {}（{}）；{credential_hits}。请先在终端运行 agy 完成 Google 登录，再点导入。",
        token_path.display(),
        if token_found { "存在" } else { "不存在" },
    )
}

fn import_account(
    state: &AppState,
    auth_payload: Value,
) -> Result<AntigravityImportOutcome, AppError> {
    let (material, email) = credential_material(&auth_payload);
    let digest = Sha256::digest(material.as_bytes());
    let fingerprint: String = digest
        .iter()
        .take(FINGERPRINT_BYTES)
        .map(|b| format!("{b:02x}"))
        .collect();
    let id = format!("{ACCOUNT_ID_PREFIX}{fingerprint}");
    let existing = state.db.get_antigravity_account(&id)?;
    let existed = existing.is_some();
    let email = email
        .filter(|s| !s.trim().is_empty())
        .or_else(|| existing.as_ref().and_then(|a| a.email.clone()));
    let name = email
        .clone()
        .filter(|e| !e.trim().is_empty())
        .or_else(|| existing.as_ref().map(|a| a.name.clone()))
        .unwrap_or_else(|| {
            let count = state
                .db
                .list_antigravity_accounts()
                .map(|list| list.len())
                .unwrap_or(0);
            format!("Google Account {}", count + 1)
        });
    state
        .db
        .upsert_antigravity_account(&id, &name, email.as_deref(), &auth_payload, true)?;
    ensure_official_current(state)?;
    crate::antigravity_config::restore_antigravity_account_live(&auth_payload)?;
    Ok(account_outcome(
        if existed {
            "account-updated"
        } else {
            "imported-account"
        },
        id,
    ))
}

#[tauri::command]
pub fn list_antigravity_accounts(
    state: State<'_, AppState>,
) -> Result<Vec<AntigravityAccount>, String> {
    state
        .db
        .list_antigravity_accounts()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn switch_antigravity_account(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    switch_account(state.inner(), &id).map_err(|e| e.to_string())
}

fn switch_account(state: &AppState, id: &str) -> Result<bool, AppError> {
    let account = state
        .db
        .get_antigravity_account(id)?
        .ok_or_else(|| AppError::Message(format!("Antigravity account '{id}' not found")))?;
    crate::antigravity_config::restore_antigravity_account_live(&account.auth_payload)?;
    state.db.set_current_antigravity_account(id)?;
    ensure_official_current(state)?;
    Ok(true)
}

#[tauri::command]
pub fn delete_antigravity_account(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    state
        .db
        .delete_antigravity_account(&id)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as BASE64;

    fn fingerprint_of(material: &str) -> String {
        let digest = Sha256::digest(material.as_bytes());
        digest
            .iter()
            .take(FINGERPRINT_BYTES)
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn material_prefers_refresh_token_from_file_token_payload() {
        let auth = serde_json::json!({
            "token": {
                "token": {
                    "access_token": "aaa",
                    "refresh_token": "rrr",
                    "expiry": "2026-01-01"
                }
            }
        });
        let (material, email) = credential_material(&auth);
        assert_eq!(material, "rrr");
        assert_eq!(email, None);

        // 指纹稳定性：access_token 轮换不改变身份
        let mut refreshed = auth.clone();
        refreshed["token"]["token"]["access_token"] = serde_json::json!("bbb");
        assert_eq!(credential_material(&refreshed).0, material);

        // refresh_token 不同 → 身份不同
        let mut other = auth.clone();
        other["token"]["token"]["refresh_token"] = serde_json::json!("zzz");
        assert_ne!(credential_material(&other).0, material);
    }

    #[test]
    fn material_decodes_credential_blob_json() {
        let token_json = serde_json::json!({
            "token": { "access_token": "a", "refresh_token": "rrr", "email": "dev@gmail.com" }
        });
        let snapshot = serde_json::json!({
            "targetName": "gemini:antigravity",
            "userName": "antigravity",
            "blob": BASE64.encode(serde_json::to_string(&token_json).unwrap()),
            "persist": 2,
            "credType": 1,
        });
        let auth = serde_json::json!({ "credential": snapshot });
        let (material, email) = credential_material(&auth);
        assert_eq!(material, "rrr");
        assert_eq!(email.as_deref(), Some("dev@gmail.com"));

        // 同一账号以文件快照导入过 → 指纹一致，去重生效
        let file_auth = serde_json::json!({ "token": token_json });
        assert_eq!(credential_material(&file_auth).0, material);
    }

    #[test]
    fn material_falls_back_to_blob_bytes_when_not_json() {
        let blob = BASE64.encode(b"\x00\x01not-json");
        let snapshot = serde_json::json!({
            "targetName": "gemini:antigravity",
            "userName": "antigravity",
            "blob": blob,
            "persist": 2,
            "credType": 1,
        });
        let auth = serde_json::json!({ "credential": snapshot });
        let (material, email) = credential_material(&auth);
        // blob 不是合法 JSON → 退回 base64 原文做指纹材料
        assert_eq!(material, blob);
        assert_eq!(email, None);
    }

    #[test]
    fn fingerprint_length_matches_design() {
        assert_eq!(fingerprint_of("anything").len(), FINGERPRINT_BYTES * 2);
    }
}
