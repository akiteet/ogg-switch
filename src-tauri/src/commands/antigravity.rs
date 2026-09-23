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

// ────────────────────────────────────────────────────────────────────────────
// CLI 强制覆盖安装（manifest 下载 → sha512 校验 → 原子替换）
//
// 场景：官方安装脚本对已安装机器是空操作（install.ps1 的 Pre-existence 分支
// 打印提示后 exit 0），`agy update` 假成功/失败时没有任何东西真的更新过二进制。
// 这条路径不问版本，直接按 manifest（version / url / sha512）把最新二进制落到
// 本地——调用方（misc.rs 的生命周期后置检查）负责判断「是否需要」。
// ────────────────────────────────────────────────────────────────────────────

const AGY_FORCE_UPDATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// 按版本号 + sha512 从 manifest 下载二进制并替换 `target`。
///
/// `client` / `target` 都可注入：测试传临时目录与本地 HTTP 服务即可覆盖整条
/// 下载-校验-替换链路，不必联网。
pub(crate) async fn force_update_agy_binary_with(
    client: &reqwest::Client,
    entry: &crate::commands::misc::AgyManifestEntry,
    target: &std::path::Path,
) -> Result<String, String> {
    use futures::StreamExt;
    use sha2::Digest as _;

    let Some(parent) = target.parent() else {
        return Err(format!("目标路径没有父目录: {}", target.display()));
    };
    std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    // 临时文件放在同一目录：rename 才能保证同盘原子替换
    let staging = parent.join(format!(
        "{}.ogg-download-{}",
        target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "agy".to_string()),
        std::process::id()
    ));

    let download = async {
        use std::io::Write as _;
        let resp = client
            .get(&entry.url)
            .timeout(AGY_FORCE_UPDATE_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("下载失败: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("下载失败: 上游返回 {}", resp.status()));
        }
        // agy 的 manifest 给的是 sha512：边落盘边算，不整块进内存
        let mut hasher = sha2::Sha512::new();
        let mut file = std::fs::File::create(&staging)
            .map_err(|e| format!("创建临时文件失败: {e}"))?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("下载中断: {e}"))?;
            std::io::Write::write_all(&mut file, &chunk)
                .map_err(|e| format!("写入临时文件失败: {e}"))?;
            sha2::Digest::update(&mut hasher, &chunk);
        }
        file.flush().ok();
        drop(file);
        Ok::<(), String>(())
    };

    if let Err(err) = download.await {
        let _ = std::fs::remove_file(&staging);
        return Err(err);
    }

    // sha512 校验：大小写不敏感地比对十六进制
    let actual = {
        let bytes = std::fs::read(&staging).map_err(|e| format!("读取临时文件失败: {e}"))?;
        let mut hasher = sha2::Sha512::new();
        sha2::Digest::update(&mut hasher, &bytes);
        hex_lower(&sha2::Digest::finalize(hasher))
    };
    if !entry.sha512.eq_ignore_ascii_case(&actual) {
        let _ = std::fs::remove_file(&staging);
        return Err(format!(
            "校验失败：下载内容与 manifest 的 sha512 不符（期望 {}，实际 {actual}），已丢弃下载",
            &entry.sha512[..16.min(entry.sha512.len())]
        ));
    }

    // 原子替换：被占用（agy 正在运行）时 rename 报错，提示先关闭
    if let Err(e) = std::fs::rename(&staging, target) {
        let _ = std::fs::remove_file(&staging);
        let busy = e.kind() == std::io::ErrorKind::PermissionDenied
            || e.raw_os_error() == Some(5);
        return Err(if busy {
            format!("替换二进制失败：目标文件被占用（agy 可能正在运行），请关闭后重试: {e}")
        } else {
            format!("替换二进制失败: {e}")
        });
    }

    // 安装器收尾（路径注册等）：失败仅记录，二进制已就位
    match crate::commands::misc::run_agy_install_setup(target) {
        Ok(()) => log::info!("agy install 收尾完成"),
        Err(err) => log::warn!("agy install 收尾失败（二进制已更新，可忽略）: {err}"),
    }

    Ok(entry.version.clone())
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ────────────────────────────────────────────────────────────────────────────
// 默认模型候选（agy models）
//
// agy 的默认模型真源是 settings.json 的 `model` 字段，合法值由 agy 自己的目录
// 决定（`agy models` 输出两列：id ↔ 显示名，实测形如
// "gemini-3.8-flash-low\tGemini 3.8 Flash (Low)"）。agy 自己写 settings.json 用
// 显示名，所以 OGG 的下拉把显示名作为存储值、id 作为辅助信息。
// ────────────────────────────────────────────────────────────────────────────

/// `agy models` 的一行候选。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityModelInfo {
    /// agy 显示名（settings.json:model 实际存储的形式，如 "Gemini 3.8 Flash (Low)"）
    pub name: String,
    /// agy 目录 id（如 "gemini-3.8-flash-low"），仅作展示辅助
    pub id: String,
}

/// 解析 `agy models` 输出：按 tab 分列的 `<id>\t<显示名>`；无 tab 的行忽略
///（banner / 进度提示都在 stderr，stdout 偶发的杂行按保守跳过处理）。
fn parse_agy_models(stdout: &str) -> Vec<AntigravityModelInfo> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let Some((id, name)) = line.split_once('\t') else {
            continue;
        };
        let id = id.trim();
        let name = name.trim();
        if id.is_empty() || name.is_empty() {
            continue;
        }
        out.push(AntigravityModelInfo {
            name: name.to_string(),
            id: id.to_string(),
        });
    }
    out
}

/// 列出 agy 目录里的默认模型候选。未装 agy / 未配凭据时返回空列表（前端
/// 保留手动输入兜底）。
#[tauri::command]
pub async fn antigravity_list_models() -> Result<Vec<AntigravityModelInfo>, String> {
    let exe = tokio::task::spawn_blocking(crate::commands::misc::locate_agy_command)
        .await
        .map_err(|e| format!("task join error: {e}"))??;
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(exe)
            .arg("models")
            .stdin(std::process::Stdio::null())
            .output()
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
    .map_err(|e| format!("执行 agy models 失败: {e}"))?;
    if !output.status.success() {
        // 典型场景：settings.json 开了 modelProvider=gemini 但没配 GEMINI_API_KEY。
        // 细节只进日志，前端按「拿不到候选」处理（保留手动输入兜底）。
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_string();
        log::warn!("agy models 非零退出: {detail}");
        return Ok(Vec::new());
    }
    Ok(parse_agy_models(&String::from_utf8_lossy(&output.stdout)))
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

    // ── manifest 覆盖安装（下载 → sha512 → 原子替换） ──────────────────────

    use crate::commands::misc::{AgyManifestEntry, agy_manifest_platform_keys};

    /// 起一个只服务一次 GET 的本地 HTTP server（与 coding_plan 测试同款，零依赖）。
    /// 返回 base_url 与线程句柄（句柄 drop 即结束）。
    fn spawn_once_server(body: &'static [u8]) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind local listener");
        let port = listener.local_addr().expect("local addr").port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body);
                let _ = stream.flush();
            }
        });
        format!("http://127.0.0.1:{port}/binary")
    }

    fn sha512_hex(payload: &[u8]) -> String {
        let mut h = sha2::Sha512::new();
        sha2::Digest::update(&mut h, payload);
        hex_lower(&sha2::Digest::finalize(h))
    }

    fn list_dir(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    /// 成功路径：目标被替换、临时文件清干净。
    #[tokio::test]
    async fn force_update_downloads_verifies_and_replaces() {
        const PAYLOAD: &[u8] = b"fake-agy-binary-v2";
        let url = spawn_once_server(PAYLOAD);
        let entry = AgyManifestEntry {
            version: "9.9.9".into(),
            url,
            sha512: sha512_hex(PAYLOAD),
        };

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("agy.exe");
        std::fs::write(&target, b"old-binary").unwrap();

        let client = reqwest::Client::new();
        let version = force_update_agy_binary_with(&client, &entry, &target)
            .await
            .expect("force update should succeed");

        assert_eq!(version, "9.9.9");
        assert_eq!(std::fs::read(&target).unwrap(), PAYLOAD);
        // 临时文件已清理：目录里只剩目标
        assert_eq!(list_dir(dir.path()), vec!["agy.exe".to_string()]);
    }

    /// sha512 不符：目标保持旧内容，下载残留被丢弃。
    #[tokio::test]
    async fn force_update_rejects_checksum_mismatch() {
        let url = spawn_once_server(b"tampered");
        let entry = AgyManifestEntry {
            version: "9.9.9".into(),
            url,
            sha512: "0".repeat(128),
        };

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("agy.exe");
        std::fs::write(&target, b"old-binary").unwrap();

        let client = reqwest::Client::new();
        let err = force_update_agy_binary_with(&client, &entry, &target)
            .await
            .expect_err("mismatched sha512 must fail");

        assert!(err.contains("校验失败"), "unexpected error: {err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"old-binary");
        assert_eq!(list_dir(dir.path()), vec!["agy.exe".to_string()]);
    }

    /// 目标被占用（模拟 agy 正在运行：Windows 上以无共享写打开锁住）→ 明确报错且
    /// 不留临时文件；目标内容不变。
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn force_update_reports_locked_target() {
        use std::os::windows::fs::OpenOptionsExt;

        const PAYLOAD: &[u8] = b"payload";
        let url = spawn_once_server(PAYLOAD);
        let entry = AgyManifestEntry {
            version: "9.9.9".into(),
            url,
            sha512: sha512_hex(PAYLOAD),
        };

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("agy.exe");
        std::fs::write(&target, b"old").unwrap();
        // share_mode(1) = 仅 FILE_SHARE_READ：锁住目标，模拟运行中的 agy.exe
        let _lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&target)
            .unwrap();

        let client = reqwest::Client::new();
        let err = force_update_agy_binary_with(&client, &entry, &target)
            .await
            .expect_err("locked target must fail");

        assert!(err.contains("替换二进制失败"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
    }

    /// 端到端（联网 + ~200MB）：真实 manifest → 下载 → 校验 → 替换到临时目录。
    /// 手动触发：cargo test -- --ignored
    #[tokio::test]
    #[ignore = "联网下载 ~200MB，仅手动验证"]
    async fn force_update_end_to_end_against_real_manifest() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .unwrap();
        let entry = crate::commands::misc::fetch_agy_manifest_entry(&client)
            .await
            .expect("real manifest should resolve");
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("agy.exe");
        let version = force_update_agy_binary_with(&client, &entry, &target)
            .await
            .expect("end-to-end force update should succeed");
        assert_eq!(version, entry.version);
        // 二进制能打印版本号
        let out = std::process::Command::new(&target)
            .arg("--version")
            .output()
            .unwrap();
        assert!(out.status.success());
        println!(
            "downloaded agy version: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }

    #[test]
    fn manifest_platform_keys_cover_current_platform() {
        // 键名与官方 install.sh/ps1 的平台字符串一致；非空即可（具体键由编译目标决定）
        assert!(!agy_manifest_platform_keys().is_empty());
    }

    // ── agy models 输出解析 ────────────────────────────────────────────────

    /// fixture 取自真机 `agy models`（1.2.8）：stdout 是 `<id>\t<显示名>` 行；
    /// "Fetching available models..." 走 stderr 不会混进来。
    #[test]
    fn parses_agy_models_catalog_lines() {
        let stdout = "gemini-3.8-flash-high\tGemini 3.8 Flash (High)\n\
                      gemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)\n\
                      gemini-3.8-flash-low\tGemini 3.8 Flash (Low)\n\
                      gemini-3.1-pro-high\tGemini 3.1 Pro (High)\n";
        let parsed = parse_agy_models(stdout);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].id, "gemini-3.8-flash-high");
        assert_eq!(parsed[0].name, "Gemini 3.8 Flash (High)");
        assert_eq!(parsed[3].name, "Gemini 3.1 Pro (High)");
    }

    #[test]
    fn parses_agy_models_skips_non_catalog_lines() {
        // 无 tab 的行（空行、横幅、杂讯）保守跳过；空字段也跳过
        let stdout = "\nAntigravity CLI 1.2.8\n\
                      gemini-3.7-flash-low\tGemini 3.7 Flash (Low)\n\
                      \t\n\
                      only-one-column\n";
        let parsed = parse_agy_models(stdout);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "gemini-3.7-flash-low");
        assert_eq!(parsed[0].name, "Gemini 3.7 Flash (Low)");
    }
}
