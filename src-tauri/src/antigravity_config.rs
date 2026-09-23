//! Antigravity CLI（agy）配置读写
//!
//! agy 是 Gemini CLI 的官方继任者（Gemini CLI 已于 2026-06-18 停止服务消费级 tier），
//! 但配置机制与 Gemini CLI 完全不同（官方文档 antigravity.google/docs/cli/install 实锤）：
//!
//! - **不加载** `~/.gemini/.env`（与 Gemini CLI 相反），`GOOGLE_API_KEY` 也无效；
//! - API key / 中转站认证需要同时具备两件事：`~/.gemini/antigravity-cli/settings.json`
//!   写入 `"modelProvider": "gemini"`（唯一合法值），再加进程环境变量
//!   `GEMINI_API_KEY`（中转站再加 `GOOGLE_GEMINI_BASE_URL`），两者缺一 CLI 无法启动；
//! - Google OAuth 登录态存于 `~/.gemini/antigravity-cli/antigravity-oauth-token`
//!   （JSON，含 token.access_token/refresh_token/expiry），多账号切换 = 快照/恢复该文件。
//!
//! 因此本模块的"live 写入" = settings.json 的 modelProvider 字段 + 持久环境变量
//! （Windows 写 HKCU\Environment，Unix 写 shell rc 受管块）+ 可选的 token 文件。

use crate::error::AppError;
use crate::provider::Provider;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// agy 认定的 API key 模式 modelProvider 值（官方文档：唯一可接受取值）
const MODEL_PROVIDER_API_KEY: &str = "gemini";

/// settings_config 的 authType 取值
pub const AUTH_TYPE_API_KEY: &str = "api-key";
pub const AUTH_TYPE_OAUTH: &str = "oauth";

/// 切换供应商时由 OGG Switch 全权托管的环境变量键。
/// 受管语义：api-key 供应商 → upsert；oauth 供应商 → 全部删除。
///
/// 历史注记：`GEMINI_MODEL` 曾在此列（Gemini CLI 惯例），但 agy 从不读它——
/// agy 的默认模型真源是 settings.json 的 `model` 字段（经 `settings_config.model`
/// 走 `update_model`）。该键已移出受管集；api-key / oauth 切换时会显式清除
/// legacy 残留。
pub const MANAGED_ENV_KEYS: [&str; 2] = ["GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL"];

/// 已废弃的 legacy 键：不再写入，但切换时主动清除（老版本 OGG 写过它）。
pub(crate) const LEGACY_ENV_KEYS: [&str; 1] = ["GEMINI_MODEL"];

// ============================================================================
// 路径
// ============================================================================

/// Antigravity CLI 配置目录（`~/.gemini/antigravity-cli`，支持设置覆盖）
pub fn get_antigravity_cli_dir() -> PathBuf {
    if let Some(custom) = crate::settings::get_antigravity_override_dir() {
        return custom;
    }
    crate::config::get_home_dir()
        .join(".gemini")
        .join("antigravity-cli")
}

/// Antigravity CLI settings.json 路径
pub fn get_antigravity_settings_path() -> PathBuf {
    get_antigravity_cli_dir().join("settings.json")
}

/// Google OAuth 登录凭据文件路径（agy 的多账号切换面）
pub fn get_antigravity_token_path() -> PathBuf {
    get_antigravity_cli_dir().join("antigravity-oauth-token")
}

// ============================================================================
// settings.json（只管 modelProvider 与 model 两个键，其余字段原样保留）
// ============================================================================

fn read_settings_json() -> Value {
    let path = get_antigravity_settings_path();
    if !path.exists() {
        return json!({});
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_else(|| {
            log::warn!(
                "Antigravity settings.json 解析失败，按空对象处理: {}",
                path.display()
            );
            json!({})
        })
}

/// 设置 / 移除 settings.json 的**单个字符串键**，保留文件中的其余字段。
///
/// `value = None` 表示删除该键；文件不存在且是删除操作时不凭空创建文件。
/// `modelProvider` 与 `model` 都走这里——agy 的「当前模型」真源就是 settings.json
/// 的 `model` 字段（agy 自己以显示名形式写入并读回，如 "Gemini 3.8 Flash (Low)"）。
fn update_settings_string_key(key: &str, value: Option<&str>) -> Result<(), AppError> {
    let path = get_antigravity_settings_path();

    let (mut settings, existed) = if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))?;
        (
            serde_json::from_str::<Value>(&content).unwrap_or_else(|_| json!({})),
            true,
        )
    } else {
        (json!({}), false)
    };

    let changed = match (value, settings.as_object_mut()) {
        (Some(v), Some(obj)) => {
            let changed = obj.get(key).and_then(Value::as_str) != Some(v);
            if changed {
                obj.insert(key.to_string(), Value::String(v.to_string()));
            }
            changed
        }
        (None, Some(obj)) => obj.remove(key).is_some(),
        // settings.json 不是对象（损坏）：仅在需要写入新值时才覆盖重建
        (Some(_), None) => true,
        (None, None) => false,
    };

    if !changed {
        return Ok(());
    }
    // 文件不存在且是删除操作 → 无需凭空创建空文件
    if !existed && value.is_none() {
        return Ok(());
    }

    crate::config::write_json_file(&path, &settings)
}

/// 设置 / 移除 settings.json 的 `modelProvider` 字段，保留文件中的其余字段。
/// `None` 表示删除该字段（回落 Google OAuth 登录态）。
pub fn update_model_provider(value: Option<&str>) -> Result<(), AppError> {
    update_settings_string_key("modelProvider", value)
}

/// 设置 / 移除 settings.json 的 `model` 字段（agy 启动时的默认模型）。
///
/// `None` 表示删除——**只在用户明确想回落 agy 自身选择时调用**；切换供应商的
/// 常规路径里「未配置默认模型」应当保留 agy 里已有的 model 不动。
pub fn update_model(value: Option<&str>) -> Result<(), AppError> {
    update_settings_string_key("model", value)
}

// ============================================================================
// 持久环境变量（受管键固定为 MANAGED_ENV_KEYS）
// ============================================================================

/// 环境变量持久层抽象：生产侧按平台实现（Windows 注册表 / Unix shell rc 块），
/// 单测注入内存实现。live 写入逻辑只对该接口编程。
pub(crate) trait PersistentEnvOps {
    fn set(&self, key: &str, value: &str) -> Result<(), AppError>;
    fn remove(&self, key: &str) -> Result<(), AppError>;
    fn get(&self, key: &str) -> Option<String>;
}

#[cfg(windows)]
mod persistent_env {
    use super::PersistentEnvOps;
    use crate::error::AppError;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    use winreg::RegKey;

    /// Windows 用户级持久环境变量：写 `HKCU\Environment` 并广播
    /// `WM_SETTINGCHANGE`，新开的进程（资源管理器派生）即可见。
    /// 已在运行的终端 / IDE 不会热更新，UI 需提示重启。
    pub(crate) struct RegistryEnv;

    fn environment_key() -> Result<RegKey, AppError> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Environment", KEY_READ | KEY_SET_VALUE)
            .map_err(|e| AppError::Message(format!("打开注册表 HKCU\\Environment 失败: {e}")))
    }

    /// 通知系统环境变量已变化（Explorer 会转发给新启动的进程）。
    fn broadcast_change() {
        use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
        };

        // lparam 需指向宽字符的 "Environment"
        let mut param: Vec<u16> = "Environment\0".encode_utf16().collect();
        let mut result: usize = 0;
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0usize as WPARAM,
                param.as_mut_ptr() as LPARAM,
                SMTO_ABORTIFHUNG,
                1000,
                &mut result,
            );
        }
    }

    impl PersistentEnvOps for RegistryEnv {
        fn set(&self, key: &str, value: &str) -> Result<(), AppError> {
            environment_key()?
                .set_value(key, &value)
                .map_err(|e| AppError::Message(format!("写入环境变量 {key} 失败: {e}")))?;
            broadcast_change();
            Ok(())
        }

        fn remove(&self, key: &str) -> Result<(), AppError> {
            match environment_key()?.delete_value(key) {
                Ok(()) => {
                    broadcast_change();
                    Ok(())
                }
                // 本来就不存在 → 目标状态已达成
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(AppError::Message(format!("删除环境变量 {key} 失败: {e}"))),
            }
        }

        fn get(&self, key: &str) -> Option<String> {
            environment_key().ok()?.get_value(key).ok()
        }
    }
}

#[cfg(not(windows))]
mod persistent_env {
    use super::rc_block;
    use super::PersistentEnvOps;
    use crate::error::AppError;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    /// Unix 持久环境变量：在 shell rc 文件（~/.zshrc、~/.bashrc）里维护
    /// OGG Switch 的标记块。所有已存在的 rc 都同步更新；一个都不存在时按
    /// 平台默认创建（macOS → ~/.zshrc，其余 → ~/.bashrc）。
    pub(crate) struct RcFileEnv;

    fn rc_candidates() -> Vec<PathBuf> {
        let home = crate::config::get_home_dir();
        vec![home.join(".zshrc"), home.join(".bashrc")]
    }

    /// 都不存在时新创建的默认 rc 文件
    pub(crate) fn default_rc_path() -> PathBuf {
        let home = crate::config::get_home_dir();
        if cfg!(target_os = "macos") {
            home.join(".zshrc")
        } else {
            home.join(".bashrc")
        }
    }

    /// 读取当前受管键值对：按候选顺序取第一个含受管块的 rc 文件
    fn read_managed_map() -> HashMap<String, String> {
        for path in rc_candidates() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(map) = rc_block::parse(&content) {
                    return map;
                }
            }
        }
        HashMap::new()
    }

    /// 把受管块写入所有已存在的 rc 文件；都不存在且块非空时创建默认 rc。
    /// 块为空 = 移除受管块（只处理已存在的文件，绝不凭空创建空块）。
    fn write_managed_map(map: &HashMap<String, String>) -> Result<(), AppError> {
        let block = if map.is_empty() {
            None
        } else {
            Some(rc_block::render(map))
        };

        let mut saw_existing = false;
        for path in rc_candidates() {
            if !path.exists() {
                continue;
            }
            saw_existing = true;
            let content = fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))?;
            let stripped = rc_block::strip(&content);
            let new_content = match &block {
                Some(block) => {
                    let mut joined = stripped;
                    if !joined.is_empty() && !joined.ends_with('\n') {
                        joined.push('\n');
                    }
                    joined.push_str(block);
                    if !joined.ends_with('\n') {
                        joined.push('\n');
                    }
                    joined
                }
                None => stripped,
            };
            if new_content != content {
                crate::config::write_text_file(&path, &new_content)?;
            }
        }

        if !saw_existing {
            if let Some(block) = block {
                let path = default_rc_path();
                let mut content = block;
                content.push('\n');
                crate::config::write_text_file(&path, &content)?;
            }
        }
        Ok(())
    }

    impl PersistentEnvOps for RcFileEnv {
        fn set(&self, key: &str, value: &str) -> Result<(), AppError> {
            let mut map = read_managed_map();
            map.insert(key.to_string(), value.to_string());
            write_managed_map(&map)
        }

        fn remove(&self, key: &str) -> Result<(), AppError> {
            let mut map = read_managed_map();
            if map.remove(key).is_none() {
                return Ok(());
            }
            write_managed_map(&map)
        }

        fn get(&self, key: &str) -> Option<String> {
            read_managed_map().get(key).cloned()
        }
    }
}

// ============================================================================
// 持久环境变量（受管键固定为 MANAGED_ENV_KEYS）
// ============================================================================

/// Windows 凭据管理器中 agy 登录凭据的快照（可序列化，随供应商条目落库）。
/// agy 在 Windows 上把 OAuth 凭据写入系统密钥环（实测目标名 `gemini:antigravity`），
/// 多账号切换 = 快照/恢复整条凭据。字段与 CREDENTIALW 一一对应，写回时原样还原。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialSnapshot {
    pub target_name: String,
    pub user_name: String,
    /// CREDENTIALW.CredentialBlob 的 base64
    pub blob: String,
    /// CREDENTIALW.Persist 原值
    pub persist: u32,
    /// CREDENTIALW.Type 原值（通常 CRED_TYPE_GENERIC = 1）
    pub cred_type: u32,
}

#[cfg(windows)]
pub mod credential_manager {
    use super::CredentialSnapshot;
    use crate::error::AppError;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Security::Credentials::{
        CredEnumerateW, CredFree, CredWriteW, CREDENTIALW,
    };

    /// agy 在 Windows 凭据管理器中的目标名（用户实测）
    const EXACT_TARGET: &str = "gemini:antigravity";
    /// ERROR_NOT_FOUND：枚举/读取无命中，属正常空结果
    const ERROR_NOT_FOUND: u32 = 1168;
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn pwstr_to_string(p: *mut u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        unsafe {
            while *p.add(len) != 0 {
                len += 1;
            }
        }
        let slice = unsafe { std::slice::from_raw_parts(p, len) };
        String::from_utf16_lossy(slice)
    }

    fn last_error(context: &str) -> AppError {
        let code = unsafe { GetLastError() };
        AppError::Message(format!("{context}失败（Windows 错误码 {code}）"))
    }

    fn snapshot_from(cred: &CREDENTIALW) -> Option<CredentialSnapshot> {
        if cred.CredentialBlob.is_null() || cred.CredentialBlobSize == 0 {
            return None;
        }
        let blob = unsafe {
            std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize)
        };
        Some(CredentialSnapshot {
            target_name: pwstr_to_string(cred.TargetName),
            user_name: pwstr_to_string(cred.UserName),
            blob: BASE64.encode(blob),
            persist: cred.Persist,
            cred_type: cred.Type,
        })
    }

    fn rank(mut candidates: Vec<CredentialSnapshot>) -> Vec<CredentialSnapshot> {
        candidates.sort_by_key(|c| {
            let target = c.target_name.to_lowercase();
            if target == EXACT_TARGET {
                0
            } else if target.contains("antigravity") {
                1
            } else {
                2
            }
        });
        candidates
    }

    fn enumerate_by_filter(filter: &str) -> Result<Vec<CredentialSnapshot>, AppError> {
        unsafe {
            let filter_wide = wide(filter);
            let mut count: u32 = 0;
            let mut creds: *mut *mut CREDENTIALW = std::ptr::null_mut();
            // windows-sys 0.61：credential 出参是 *mut *mut *mut CREDENTIALW
            if CredEnumerateW(filter_wide.as_ptr(), 0, &mut count, &mut creds) == 0 {
                if GetLastError() == ERROR_NOT_FOUND {
                    return Ok(Vec::new());
                }
                return Err(last_error("枚举凭据管理器"));
            }
            let mut out = Vec::new();
            for i in 0..count as usize {
                if let Some(snap) = snapshot_from(&*(*creds.add(i))) {
                    out.push(snap);
                }
            }
            CredFree(creds as *const core::ffi::c_void);
            Ok(out)
        }
    }

    /// 枚举凭据管理器中与 agy 相关的凭据，按命中优先级排序（精确 → antigravity → gemini）
    /// 枚举凭据管理器中与 agy 相关的凭据，按命中优先级排序。
    ///
    /// 注意 CredEnumerateW 的过滤语义：**只支持前缀通配**（`gemini:*` 命中，
    /// `*gemini*` 会返回 ERROR_NOT_FOUND），因此前缀快速路径之外必须用 `*`
    /// 全量枚举后在 Rust 侧做包含匹配兜底。
    pub fn find_matching() -> Result<Vec<CredentialSnapshot>, AppError> {
        // 快速路径：agy 的目标名前缀（实测 gemini:antigravity）
        if let Ok(candidates) = enumerate_by_filter("gemini:*") {
            if !candidates.is_empty() {
                return Ok(rank(candidates));
            }
        }
        // 兜底：全量枚举后包含匹配（目标名可能带其他前缀形态）
        let all = enumerate_by_filter("*")?;
        let candidates: Vec<CredentialSnapshot> = all
            .into_iter()
            .filter(|c| {
                let target = c.target_name.to_lowercase();
                target.contains("antigravity")
                    || target.contains("gemini")
                    || target.contains("agy")
            })
            .collect();
        if !candidates.is_empty() {
            return Ok(rank(candidates));
        }
        Ok(Vec::new())
    }

    pub fn write(snapshot: &CredentialSnapshot) -> Result<(), AppError> {
        let blob = BASE64
            .decode(snapshot.blob.as_bytes())
            .map_err(|e| AppError::Message(format!("凭据快照 base64 解码失败: {e}")))?;
        let mut target_wide = wide(&snapshot.target_name);
        let mut user_wide = wide(&snapshot.user_name);
        let cred = CREDENTIALW {
            Type: snapshot.cred_type,
            TargetName: target_wide.as_mut_ptr(),
            UserName: user_wide.as_mut_ptr(),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_ptr() as *mut u8,
            Persist: snapshot.persist,
            ..Default::default()
        };
        unsafe {
            if CredWriteW(&cred, 0) == 0 {
                return Err(last_error("写回凭据管理器"));
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
pub mod credential_manager {
    use super::CredentialSnapshot;
    use crate::error::AppError;

    /// 非 Windows 平台无凭据管理器路径（Unix 走 token 文件快照）
    pub fn find_matching() -> Result<Vec<CredentialSnapshot>, AppError> {
        Ok(Vec::new())
    }

    pub fn read(_target_name: &str) -> Result<Option<CredentialSnapshot>, AppError> {
        Ok(None)
    }

    pub fn write(_snapshot: &CredentialSnapshot) -> Result<(), AppError> {
        Err(AppError::Message(
            "该账号快照来自 Windows 凭据管理器，当前平台无法恢复".to_string(),
        ))
    }
}

// ============================================================================
// shell rc 受管块（纯函数，跨平台编译以便统一测试）
// ============================================================================

/// `~/.zshrc` / `~/.bashrc` 受管块的渲染、剥离与解析。
/// 块形如：
/// ```text
/// # >>> OGG-Switch (antigravity) >>>
/// export GEMINI_API_KEY='...'
/// # <<< OGG-Switch (antigravity) <<<
/// ```
/// 跨平台编译以便统一单测；Windows 生产构建不使用（注册表路径），故允许 dead_code。
#[cfg_attr(windows, allow(dead_code))]
pub(crate) mod rc_block {
    use super::MANAGED_ENV_KEYS;
    use std::collections::HashMap;

    pub(crate) const BLOCK_BEGIN: &str = "# >>> OGG-Switch (antigravity) >>>";
    pub(crate) const BLOCK_END: &str = "# <<< OGG-Switch (antigravity) <<<";

    /// 单引号转义（shell 语义）：`'` → `'\''`
    pub(crate) fn escape_single_quoted(value: &str) -> String {
        format!("'{}'", value.replace('\'', r"'\''"))
    }

    /// 上述转义的逆运算（容忍未加引号的裸值）
    pub(crate) fn unescape_single_quoted(raw: &str) -> String {
        let trimmed = raw.trim();
        let inner = trimmed
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .unwrap_or(trimmed);
        inner.replace(r"'\''", "'")
    }

    /// 渲染受管块；受管键按 MANAGED_ENV_KEYS 顺序输出，其余键按字典序追加
    pub(crate) fn render(map: &HashMap<String, String>) -> String {
        let mut lines = vec![BLOCK_BEGIN.to_string()];
        for key in MANAGED_ENV_KEYS {
            if let Some(value) = map.get(key) {
                lines.push(format!("export {key}={}", escape_single_quoted(value)));
            }
        }
        let mut extra: Vec<(&String, &String)> = map
            .iter()
            .filter(|(k, _)| !MANAGED_ENV_KEYS.contains(&k.as_str()))
            .collect();
        extra.sort_by(|a, b| a.0.cmp(b.0));
        for (key, value) in extra {
            lines.push(format!("export {key}={}", escape_single_quoted(value)));
        }
        lines.push(BLOCK_END.to_string());
        lines.join("\n")
    }

    /// 从文件内容剥离受管块，返回其余内容（保留原有换行结构）
    pub(crate) fn strip(content: &str) -> String {
        let mut kept: Vec<&str> = Vec::new();
        let mut inside = false;
        for line in content.split('\n') {
            let trimmed = line.trim_end_matches('\r');
            if trimmed == BLOCK_BEGIN {
                inside = true;
                continue;
            }
            if inside {
                if trimmed == BLOCK_END {
                    inside = false;
                }
                continue;
            }
            kept.push(line);
        }
        kept.join("\n")
    }

    /// 解析第一个受管块中的键值对；无块返回 None（残缺块也按已找到处理）
    pub(crate) fn parse(content: &str) -> Option<HashMap<String, String>> {
        let mut inside = false;
        let mut map = HashMap::new();
        for line in content.split('\n') {
            let trimmed = line.trim_end_matches('\r').trim();
            if trimmed == BLOCK_BEGIN {
                inside = true;
                continue;
            }
            if trimmed == BLOCK_END {
                if inside {
                    return Some(map);
                }
                continue;
            }
            if inside {
                if let Some(rest) = trimmed.strip_prefix("export ") {
                    if let Some((key, value)) = rest.split_once('=') {
                        let key = key.trim();
                        if !key.is_empty() {
                            map.insert(key.to_string(), unescape_single_quoted(value));
                        }
                    }
                }
            }
        }
        inside.then_some(map)
    }
}

// ============================================================================
// live 快照
// ============================================================================

/// 读 token 文件：缺失 → Ok(None)；损坏 → Err（导入流程据此给明确报错）
pub fn read_token_file() -> Result<Option<Value>, AppError> {
    let path = get_antigravity_token_path();
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))?;
    serde_json::from_str::<Value>(&content).map(Some).map_err(|err| {
        AppError::localized(
            "antigravity.token.corrupt",
            format!(
                "Antigravity 登录凭据文件损坏（不是合法 JSON）：{}\n解析错误：{err}\n可删除该文件后在终端重新运行 agy 登录。",
                path.display()
            ),
            format!(
                "Antigravity credential file is corrupted (invalid JSON): {}\nParse error: {err}\nDelete it and run `agy` login in a terminal to re-authenticate.",
                path.display()
            ),
        )
    })
}

/// 发现本地 agy 登录凭据：优先 token 文件，其次 Windows 凭据管理器。
/// 返回值直接作为 settings_config 的 auth 载荷：`{"token": …}`（文件快照）
/// 或 `{"credential": …}`（Windows 凭据快照）。
pub fn discover_local_auth() -> Result<Option<Value>, AppError> {
    if let Some(token) = read_token_file()? {
        return Ok(Some(json!({ "token": token })));
    }
    let candidates = credential_manager::find_matching()?;
    if let Some(snap) = candidates.first() {
        return Ok(Some(json!({ "credential": snap })));
    }
    Ok(None)
}

/// 组装 live 快照（与 Provider.settings_config 同形），输入显式注入以便单测
pub(crate) fn compose_live_snapshot(
    settings_json: &Value,
    managed_env: &HashMap<String, String>,
    auth_payload: Option<&Value>,
) -> Value {
    let model_provider = settings_json
        .get("modelProvider")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let api_key = managed_env
        .get("GEMINI_API_KEY")
        .map(String::as_str)
        .unwrap_or_default();

    // api-key 态判定：settings.json 声明 modelProvider=gemini，或持久 env 里有 key
    if model_provider == MODEL_PROVIDER_API_KEY || !api_key.is_empty() {
        let mut env = Map::new();
        for key in MANAGED_ENV_KEYS {
            if let Some(value) = managed_env
                .get(key)
                .map(String::as_str)
                .filter(|s| !s.is_empty())
            {
                env.insert(key.to_string(), Value::String(value.to_string()));
            }
        }
        // agy 当前模型（settings.json:model，agy 自己写显示名）随快照带出：
        // EditProviderDialog 编辑当前供应商时会整体替换 settingsConfig，
        // 不带这个字段的话表单会把已生效的默认模型显示成空。
        let current_model = settings_json
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let mut snapshot = json!({ "authType": AUTH_TYPE_API_KEY, "env": Value::Object(env) });
        if let Some(model) = current_model {
            if let Some(obj) = snapshot.as_object_mut() {
                obj.insert("model".to_string(), Value::String(model.to_string()));
            }
        }
        return snapshot;
    }

    let mut snapshot = json!({ "authType": AUTH_TYPE_OAUTH });
    if let Some(auth) = auth_payload {
        // auth_payload 形如 {"token": …} / {"credential": …}，原样并入快照
        if let (Some(target), Some(source)) = (snapshot.as_object_mut(), auth.as_object()) {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
    }
    snapshot
}

fn managed_env_from_ops(env_ops: &dyn PersistentEnvOps) -> HashMap<String, String> {
    MANAGED_ENV_KEYS
        .iter()
        .filter_map(|key| env_ops.get(key).map(|value| (key.to_string(), value)))
        .collect()
}

/// 读取当前 live 状态快照。凭据损坏时按无凭据处理（只 warn，不影响读）。
pub fn read_antigravity_live_settings() -> Result<Value, AppError> {
    let settings_json = read_settings_json();
    let managed_env = managed_env_from_ops(persistent_env_impl());
    let auth_payload = match discover_local_auth() {
        Ok(auth) => auth,
        Err(e) => {
            log::warn!("读取 Antigravity 登录凭据失败，按无凭据处理: {e}");
            None
        }
    };
    Ok(compose_live_snapshot(
        &settings_json,
        &managed_env,
        auth_payload.as_ref(),
    ))
}

/// live 是否处于 API key / 中转站态（vs Google 登录态）
pub fn is_api_key_live() -> bool {
    matches!(
        read_antigravity_live_settings(),
        Ok(snapshot)
            if snapshot.get("authType").and_then(Value::as_str) == Some(AUTH_TYPE_API_KEY)
    )
}

// ============================================================================
// 供应商 → live 写入
// ============================================================================

fn persistent_env_impl() -> &'static dyn PersistentEnvOps {
    #[cfg(windows)]
    {
        static IMPL: persistent_env::RegistryEnv = persistent_env::RegistryEnv;
        &IMPL
    }
    #[cfg(not(windows))]
    {
        static IMPL: persistent_env::RcFileEnv = persistent_env::RcFileEnv;
        &IMPL
    }
}

fn write_token_file(token: &Value) -> Result<(), AppError> {
    let path = get_antigravity_token_path();
    let content = serde_json::to_string_pretty(token)
        .map_err(|e| AppError::Message(format!("Antigravity token 序列化失败: {e}")))?;
    crate::config::write_text_file(&path, &content)?;

    // 凭据文件收敛到仅所有者可读写
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)
            .map_err(|e| AppError::io(&path, e))?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms).map_err(|e| AppError::io(&path, e))?;
    }
    Ok(())
}

/// 把供应商写入 live 配置（核心写入点，挂接 `write_live_snapshot`）
pub fn write_antigravity_provider_live(provider: &Provider) -> Result<(), AppError> {
    write_antigravity_provider_live_with(provider, persistent_env_impl())
}

/// 变更顺序刻意安排成"先改环境侧、最后动 settings.json 开关"，任何一步失败
/// 都不会把 agy 留在 `modelProvider=gemini` 而没有 GEMINI_API_KEY 的起不来状态。
pub(crate) fn write_antigravity_provider_live_with(
    provider: &Provider,
    env_ops: &dyn PersistentEnvOps,
) -> Result<(), AppError> {
    let auth_type = provider
        .settings_config
        .get("authType")
        .and_then(Value::as_str)
        .unwrap_or(AUTH_TYPE_API_KEY);

    match auth_type {
        AUTH_TYPE_OAUTH => {
            let token = provider.settings_config.get("token");
            let credential = provider.settings_config.get("credential");
            if let Some(token) = token {
                if !(token.is_object() || token.is_null()) {
                    return Err(AppError::localized(
                        "antigravity.validation.invalid_token",
                        "Antigravity 配置格式错误: token 必须是对象",
                        "Antigravity config invalid: token must be an object",
                    ));
                }
            }
            if let Some(credential) = credential {
                if !(credential.is_object() || credential.is_null()) {
                    return Err(AppError::localized(
                        "antigravity.validation.invalid_credential",
                        "Antigravity 配置格式错误: credential 必须是对象",
                        "Antigravity config invalid: credential must be an object",
                    ));
                }
            }

            // 1. 清 API key 痕迹（含 legacy GEMINI_MODEL）2. 恢复登录凭据（文件 /
            // Windows 凭据管理器）3. 摘掉 modelProvider 开关。官方条目（两者皆无）
            // 不接管 agy 自身登录态。不动 settings.json 的 model——那是 agy 全局
            // 偏好，不属于凭据面。
            for key in MANAGED_ENV_KEYS {
                env_ops.remove(key)?;
            }
            for legacy in LEGACY_ENV_KEYS {
                env_ops.remove(legacy)?;
            }
            let has_file_token =
                token.is_some_and(|t| t.as_object().is_some_and(|obj| !obj.is_empty()));
            if has_file_token {
                write_token_file(token.expect("checked above"))?;
            }
            if let Some(credential) = credential.filter(|c| c.is_object()) {
                let snapshot: CredentialSnapshot = serde_json::from_value(credential.clone())
                    .map_err(|e| AppError::Message(format!("Antigravity 凭据快照解析失败: {e}")))?;
                credential_manager::write(&snapshot)?;
            }
            update_model_provider(None)?;
            Ok(())
        }
        _ => {
            let env = provider
                .settings_config
                .get("env")
                .and_then(Value::as_object);
            let api_key = env
                .and_then(|e| e.get("GEMINI_API_KEY"))
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if api_key.is_empty() {
                return Err(AppError::localized(
                    "antigravity.validation.missing_api_key",
                    "Antigravity 配置缺少必需字段: GEMINI_API_KEY（agy 设置 modelProvider=gemini 后没有它将无法启动）",
                    "Antigravity config missing required field: GEMINI_API_KEY (agy won't start with modelProvider=gemini without it)",
                ));
            }

            // 1. 全量 upsert env（额外变量随供应商落持久环境变量；
            //    空值 = 删除该键，GOOGLE_GEMINI_BASE_URL 由专属字段控制）
            // 2. legacy 清理：老版本 OGG 曾写 GEMINI_MODEL（agy 从不读它）
            // 3. 默认模型写 settings.json 的 model（agy 的真源）；未配置则不动——
            //    保留用户在 agy 里 /model 选过的值
            // 4. 最后打开 modelProvider 开关
            let has_base_url = env
                .and_then(|e| e.get("GOOGLE_GEMINI_BASE_URL"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            for (key, value) in env.iter().flat_map(|obj| obj.iter()) {
                let key_str = key.as_str();
                if key_str.is_empty()
                    || !key_str
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    continue;
                }
                let value_str = value.as_str().map(str::trim).unwrap_or_default();
                if value_str.is_empty() {
                    env_ops.remove(key_str)?;
                } else {
                    env_ops.set(key_str, value_str)?;
                }
            }
            // 切走中转站时清掉残留端点：env 没带 BASE_URL（或为空）→ 删除
            if has_base_url.is_none() {
                env_ops.remove("GOOGLE_GEMINI_BASE_URL")?;
            }
            for legacy in LEGACY_ENV_KEYS {
                env_ops.remove(legacy)?;
            }

            // 默认模型：settings_config 顶层的 model 字段（非 env！）
            let configured_model = provider
                .settings_config
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if let Some(model) = configured_model {
                update_model(Some(model))?;
            }

            update_model_provider(Some(MODEL_PROVIDER_API_KEY))?;
            Ok(())
        }
    }
}

/// 把账号池快照写回 agy live 凭据（token 文件 / Windows 凭据管理器），
/// 并清掉 API key 环境与 `modelProvider`。不创建、不切换供应商。
pub fn restore_antigravity_account_live(auth_payload: &Value) -> Result<(), AppError> {
    restore_antigravity_account_live_with(auth_payload, persistent_env_impl())
}

pub(crate) fn restore_antigravity_account_live_with(
    auth_payload: &Value,
    env_ops: &dyn PersistentEnvOps,
) -> Result<(), AppError> {
    for key in MANAGED_ENV_KEYS {
        env_ops.remove(key)?;
    }
    let has_file_token = auth_payload
        .get("token")
        .and_then(Value::as_object)
        .is_some_and(|obj| !obj.is_empty());
    if has_file_token {
        write_token_file(auth_payload.get("token").expect("checked above"))?;
    } else {
        // 纯凭据管理器快照：清掉残留 token 文件，避免下次发现优先读到旧文件。
        let path = get_antigravity_token_path();
        if path.exists() {
            fs::remove_file(&path).map_err(|e| AppError::io(&path, e))?;
        }
    }
    if let Some(credential) = auth_payload.get("credential").filter(|c| c.is_object()) {
        let snapshot: CredentialSnapshot = serde_json::from_value(credential.clone())
            .map_err(|e| AppError::Message(format!("Antigravity 凭据快照解析失败: {e}")))?;
        credential_manager::write(&snapshot)?;
    } else if has_file_token {
        if let Some(snapshot) =
            credential_snapshot_from_token(auth_payload.get("token").expect("checked above"))
        {
            credential_manager::write(&snapshot)?;
        }
    }
    update_model_provider(None)?;
    Ok(())
}

fn credential_snapshot_from_token(token: &Value) -> Option<CredentialSnapshot> {
    let blob = serde_json::to_vec(&json!({ "token": token, "auth_method": "consumer" })).ok()?;
    Some(CredentialSnapshot {
        target_name: "gemini:antigravity".to_string(),
        user_name: "antigravity".to_string(),
        blob: {
            use base64::engine::general_purpose::STANDARD as BASE64;
            use base64::Engine as _;
            BASE64.encode(blob)
        },
        persist: 2,
        cred_type: 1,
    })
}

// ============================================================================
// Google userinfo（仅在导入已有凭据时补全显示名）
// ============================================================================

const USERINFO_ENDPOINT: &str = "https://www.googleapis.com/oauth2/v2/userinfo";

#[derive(Debug, serde::Deserialize)]
struct GoogleUserInfo {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

/// 用 access_token 查 Google userinfo 取邮箱（导入已有凭据时补全显示名）。
///
/// 只发起一次只读查询，不涉及任何 OAuth client 凭据。
pub async fn fetch_email_for_access_token(access_token: &str) -> Option<String> {
    let response = crate::proxy::http_client::get()
        .get(USERINFO_ENDPOINT)
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let info: GoogleUserInfo = response.json().await.ok()?;
    info.email
        .filter(|s| !s.is_empty())
        .or(info.name.filter(|s| s.contains('@')))
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    /// 隔离的临时 HOME（保存并恢复 CC_SWITCH_TEST_HOME）
    fn with_test_home<T>(test_fn: impl FnOnce(&std::path::Path) -> T) -> T {
        let _guard = test_guard();
        let tmp = tempfile::tempdir().unwrap();
        let old_test_home = std::env::var_os("CC_SWITCH_TEST_HOME");
        std::env::set_var("CC_SWITCH_TEST_HOME", tmp.path());
        let result = test_fn(tmp.path());
        match old_test_home {
            Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
            None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
        }
        result
    }

    /// 内存版环境变量持久层（隔离测试用，绝不触碰真实注册表 / rc 文件）
    struct MemEnv(std::sync::Mutex<HashMap<String, String>>);

    impl MemEnv {
        fn new() -> Self {
            Self(std::sync::Mutex::new(HashMap::new()))
        }

        fn insert(&self, key: &str, value: &str) {
            self.0
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
        }
    }

    impl PersistentEnvOps for MemEnv {
        fn set(&self, key: &str, value: &str) -> Result<(), AppError> {
            self.insert(key, value);
            Ok(())
        }

        fn remove(&self, key: &str) -> Result<(), AppError> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }

        fn get(&self, key: &str) -> Option<String> {
            self.0.lock().unwrap().get(key).cloned()
        }
    }

    fn provider_of(settings_config: Value) -> Provider {
        Provider::with_id(
            "test-antigravity".to_string(),
            "Test Antigravity".to_string(),
            settings_config,
            None,
        )
    }

    // ---- rc_block 纯函数 ----

    #[test]
    fn rc_block_escape_roundtrip() {
        let value = "it's a 'quoted' key";
        let escaped = rc_block::escape_single_quoted(value);
        assert_eq!(escaped, r"'it'\''s a '\''quoted'\'' key'");
        assert_eq!(rc_block::unescape_single_quoted(&escaped), value);
        // 裸值容忍
        assert_eq!(rc_block::unescape_single_quoted("plain"), "plain");
    }

    #[test]
    fn rc_block_render_strip_parse_roundtrip() {
        let mut map = HashMap::new();
        map.insert("GEMINI_API_KEY".to_string(), "sk-test".to_string());
        map.insert(
            "GOOGLE_GEMINI_BASE_URL".to_string(),
            "https://relay.example.com/gemini".to_string(),
        );

        let block = rc_block::render(&map);
        assert!(block.starts_with(rc_block::BLOCK_BEGIN));
        assert!(block.ends_with(rc_block::BLOCK_END));
        assert!(block.contains("export GEMINI_API_KEY='sk-test'"));

        let wrapped = format!(
            "# user profile\nalias ll='ls -la'\n{}\n# after block\n",
            block
        );
        let parsed = rc_block::parse(&wrapped).expect("block parsed");
        assert_eq!(parsed.get("GEMINI_API_KEY").unwrap(), "sk-test");
        assert_eq!(
            parsed.get("GOOGLE_GEMINI_BASE_URL").unwrap(),
            "https://relay.example.com/gemini"
        );

        let stripped = rc_block::strip(&wrapped);
        assert!(!stripped.contains("GEMINI_API_KEY"));
        assert!(stripped.contains("alias ll='ls -la'"));
        assert!(stripped.contains("# after block"));
        assert!(rc_block::parse(&stripped).is_none());
    }

    #[test]
    fn rc_block_parse_partial_block_still_found() {
        // 缺 END 标记的残缺块也按已找到处理，避免剥离时漏清
        let content = format!("{}\nexport GEMINI_API_KEY='k'", rc_block::BLOCK_BEGIN);
        let parsed = rc_block::parse(&content).expect("partial block parsed");
        assert_eq!(parsed.get("GEMINI_API_KEY").unwrap(), "k");
    }

    #[cfg(not(windows))]
    #[test]
    #[serial_test::serial]
    fn rc_file_env_write_and_cleanup() {
        with_test_home(|home| {
            let zshrc = home.join(".zshrc");
            std::fs::write(&zshrc, "export EDITOR=vim\n").unwrap();

            let ops = persistent_env::RcFileEnv;
            ops.set("GEMINI_API_KEY", "sk-live").unwrap();
            ops.set("GOOGLE_GEMINI_BASE_URL", "https://relay.example.com")
                .unwrap();

            let content = std::fs::read_to_string(&zshrc).unwrap();
            assert!(content.contains("export EDITOR=vim"));
            assert!(content.contains("export GEMINI_API_KEY='sk-live'"));
            assert_eq!(ops.get("GEMINI_API_KEY").as_deref(), Some("sk-live"));

            // 覆盖更新
            ops.set("GEMINI_API_KEY", "sk-updated").unwrap();
            let content = std::fs::read_to_string(&zshrc).unwrap();
            assert!(content.contains("sk-updated"));
            assert!(!content.contains("sk-live"));

            // 全部移除后块整体消失，原有内容保留
            ops.remove("GEMINI_API_KEY").unwrap();
            ops.remove("GOOGLE_GEMINI_BASE_URL").unwrap();
            let content = std::fs::read_to_string(&zshrc).unwrap();
            assert!(content.contains("export EDITOR=vim"));
            assert!(!content.contains(rc_block::BLOCK_BEGIN));
            assert_eq!(ops.get("GEMINI_API_KEY"), None);

            // rc 都不存在时写入 → 创建平台默认 rc
            let bashrc = home.join(".bashrc");
            let _ = std::fs::remove_file(&bashrc);
            let _ = std::fs::remove_file(&zshrc);
            ops.set("GEMINI_API_KEY", "sk-fresh").unwrap();
            assert!(persistent_env::default_rc_path().exists());
        });
    }

    // ---- settings.json modelProvider 管理 ----

    #[test]
    #[serial_test::serial]
    fn update_model_provider_preserves_other_fields() {
        with_test_home(|_home| {
            let path = get_antigravity_settings_path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                r#"{"modelProvider":"gemini","theme":"dark","keybindings":{}}"#,
            )
            .unwrap();

            update_model_provider(None).unwrap();
            let value: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert!(value.get("modelProvider").is_none());
            assert_eq!(value.get("theme").and_then(Value::as_str), Some("dark"));
            assert!(value.get("keybindings").is_some());

            update_model_provider(Some("gemini")).unwrap();
            let value: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(
                value.get("modelProvider").and_then(Value::as_str),
                Some("gemini")
            );
            assert_eq!(value.get("theme").and_then(Value::as_str), Some("dark"));
        });
    }

    #[test]
    #[serial_test::serial]
    fn update_model_provider_removal_without_file_is_noop() {
        with_test_home(|_home| {
            let path = get_antigravity_settings_path();
            let _ = std::fs::remove_file(&path);
            update_model_provider(None).unwrap();
            assert!(!path.exists());
            // 设置值则会创建文件
            update_model_provider(Some("gemini")).unwrap();
            assert!(path.exists());
        });
    }

    // ---- live 快照组装 ----

    #[test]
    fn snapshot_api_key_mode_from_model_provider() {
        let mut env = HashMap::new();
        env.insert("GEMINI_API_KEY".to_string(), "sk-relay".to_string());
        let snapshot = compose_live_snapshot(
            &json!({"modelProvider": "gemini", "theme": "dark"}),
            &env,
            None,
        );
        assert_eq!(snapshot["authType"], AUTH_TYPE_API_KEY);
        assert_eq!(snapshot["env"]["GEMINI_API_KEY"], "sk-relay");
        assert!(snapshot.get("token").is_none());
    }

    #[test]
    fn snapshot_api_key_mode_from_env_without_model_provider() {
        let mut env = HashMap::new();
        env.insert("GEMINI_API_KEY".to_string(), "sk-x".to_string());
        let snapshot = compose_live_snapshot(&json!({}), &env, Some(&json!({"stale": true})));
        // env 已声明 key → api-key 态优先，token 快照不应出现
        assert_eq!(snapshot["authType"], AUTH_TYPE_API_KEY);
        assert!(snapshot.get("token").is_none());
    }

    #[test]
    fn snapshot_oauth_mode_with_and_without_token() {
        // modelProvider=gemini 且无 key：仍按 api-key 判定（开关优先）
        let snapshot = compose_live_snapshot(
            &json!({"modelProvider": "gemini"}),
            &HashMap::new(),
            Some(&json!({"token": {"access_token": "a"}})),
        );
        assert_eq!(snapshot["authType"], AUTH_TYPE_API_KEY);
        assert!(snapshot["env"]["GEMINI_API_KEY"].is_null());

        let snapshot = compose_live_snapshot(&json!({}), &HashMap::new(), None);
        assert_eq!(snapshot["authType"], AUTH_TYPE_OAUTH);
        assert!(snapshot.get("token").is_none());

        let token = json!({"token": {"access_token": "a"}});
        let snapshot = compose_live_snapshot(&json!({}), &HashMap::new(), Some(&token));
        assert_eq!(snapshot["authType"], AUTH_TYPE_OAUTH);
        // auth_payload 的键原样并入快照（{"token": …} → snapshot.token）
        assert_eq!(snapshot["token"]["access_token"], "a");

        let cred = json!({"credential": {"targetName": "gemini:antigravity"}});
        let snapshot = compose_live_snapshot(&json!({}), &HashMap::new(), Some(&cred));
        assert_eq!(snapshot["authType"], AUTH_TYPE_OAUTH);
        assert_eq!(snapshot["credential"]["targetName"], "gemini:antigravity");
    }

    // ---- live 写入（注入内存 env 层）----

    #[test]
    #[serial_test::serial]
    fn write_api_key_provider_sets_env_then_switch() {
        with_test_home(|_home| {
            let mem = MemEnv::new();
            let provider = provider_of(json!({
                "authType": "api-key",
                "env": {
                    "GEMINI_API_KEY": "sk-relay",
                    "GOOGLE_GEMINI_BASE_URL": "https://relay.example.com/gemini"
                }
            }));
            write_antigravity_provider_live_with(&provider, &mem).unwrap();

            assert_eq!(mem.get("GEMINI_API_KEY").as_deref(), Some("sk-relay"));
            assert_eq!(
                mem.get("GOOGLE_GEMINI_BASE_URL").as_deref(),
                Some("https://relay.example.com/gemini")
            );
            let value: Value = serde_json::from_str(
                &std::fs::read_to_string(get_antigravity_settings_path()).unwrap(),
            )
            .unwrap();
            assert_eq!(
                value.get("modelProvider").and_then(Value::as_str),
                Some("gemini")
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn write_api_key_without_base_url_clears_stale_endpoint() {
        with_test_home(|_home| {
            let mem = MemEnv::new();
            mem.insert("GOOGLE_GEMINI_BASE_URL", "https://old.example.com");

            let provider = provider_of(json!({
                "authType": "api-key",
                "env": { "GEMINI_API_KEY": "sk-official" }
            }));
            write_antigravity_provider_live_with(&provider, &mem).unwrap();

            assert_eq!(mem.get("GEMINI_API_KEY").as_deref(), Some("sk-official"));
            assert_eq!(mem.get("GOOGLE_GEMINI_BASE_URL"), None);
        });
    }

    #[test]
    fn write_api_key_missing_key_is_rejected() {
        let mem = MemEnv::new();
        let provider = provider_of(json!({ "authType": "api-key", "env": {} }));
        let err = write_antigravity_provider_live_with(&provider, &mem)
            .expect_err("missing api key must fail");
        assert!(err.to_string().contains("GEMINI_API_KEY"));
        // 未产生任何持久化副作用
        assert_eq!(mem.get("GEMINI_API_KEY"), None);
    }

    #[test]
    #[serial_test::serial]
    fn write_oauth_account_writes_token_and_clears_env() {
        with_test_home(|_home| {
            let mem = MemEnv::new();
            mem.insert("GEMINI_API_KEY", "sk-stale");
            let settings_path = get_antigravity_settings_path();
            std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
            std::fs::write(&settings_path, r#"{"modelProvider":"gemini"}"#).unwrap();

            let token = json!({"token": {"access_token": "a", "refresh_token": "r", "expiry": "2026-01-01"}});
            let provider = provider_of(json!({ "authType": "oauth", "token": token }));
            write_antigravity_provider_live_with(&provider, &mem).unwrap();

            assert_eq!(mem.get("GEMINI_API_KEY"), None);
            assert_eq!(mem.get("GOOGLE_GEMINI_BASE_URL"), None);
            let value: Value =
                serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
            assert!(value.get("modelProvider").is_none());

            let saved: Value = serde_json::from_str(
                &std::fs::read_to_string(get_antigravity_token_path()).unwrap(),
            )
            .unwrap();
            assert_eq!(saved["token"]["refresh_token"], "r");
        });
    }

    #[test]
    #[serial_test::serial]
    fn write_oauth_official_never_touches_token_file() {
        with_test_home(|_home| {
            let mem = MemEnv::new();
            // agy 自身登录态已存在
            let token_path = get_antigravity_token_path();
            std::fs::create_dir_all(token_path.parent().unwrap()).unwrap();
            std::fs::write(&token_path, r#"{"token":{"access_token":"live"}}"#).unwrap();

            let provider = provider_of(json!({ "authType": "oauth" }));
            write_antigravity_provider_live_with(&provider, &mem).unwrap();

            assert_eq!(mem.get("GEMINI_API_KEY"), None);
            // 官方条目不接管 token 文件
            let content = std::fs::read_to_string(&token_path).unwrap();
            assert!(content.contains("live"));
            // 本用例未预置 settings.json：官方分支只做"删除"语义，不应凭空创建
            let settings: Option<Value> = std::fs::read_to_string(get_antigravity_settings_path())
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok());
            assert!(
                settings
                    .as_ref()
                    .and_then(|value| value.get("modelProvider"))
                    .is_none(),
                "modelProvider must be absent: {settings:?}"
            );
        });
    }

    // ---- token 文件读取 ----

    #[test]
    #[serial_test::serial]
    fn read_token_file_missing_and_corrupt() {
        with_test_home(|_home| {
            assert!(read_token_file().unwrap().is_none());

            let token_path = get_antigravity_token_path();
            std::fs::create_dir_all(token_path.parent().unwrap()).unwrap();
            std::fs::write(&token_path, "not-json").unwrap();
            assert!(read_token_file().is_err());
        });
    }

    // ---- 默认模型（settings.json:model）----

    #[test]
    #[serial_test::serial]
    fn update_model_preserves_other_fields() {
        with_test_home(|_home| {
            let path = get_antigravity_settings_path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                r#"{"modelProvider":"gemini","model":"Gemini 3.8 Flash (Low)","colorScheme":"dark"}"#,
            )
            .unwrap();

            update_model(Some("Gemini 3.1 Pro (High)")).unwrap();
            let value: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(
                value.get("model").and_then(Value::as_str),
                Some("Gemini 3.1 Pro (High)")
            );
            assert_eq!(
                value.get("modelProvider").and_then(Value::as_str),
                Some("gemini")
            );
            assert_eq!(value.get("colorScheme").and_then(Value::as_str), Some("dark"));

            update_model(None).unwrap();
            let value: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert!(value.get("model").is_none());
            assert_eq!(
                value.get("modelProvider").and_then(Value::as_str),
                Some("gemini")
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn api_key_switch_writes_model_and_clears_legacy_env() {
        with_test_home(|home| {
            // 预置 legacy 残留：老版本 OGG 写过 GEMINI_MODEL
            let mem = MemEnv::new();
            mem.set("GEMINI_MODEL", "gemini-3.1-pro-preview").unwrap();

            let provider = provider_of(serde_json::json!({
                "authType": "api-key",
                // 顶层 model 字段（非 env！）
                "model": "Gemini 3.8 Flash (Low)",
                "env": { "GEMINI_API_KEY": "sk-test", "GOOGLE_GEMINI_BASE_URL": "https://relay.example.com" }
            }));
            write_antigravity_provider_live_with(&provider, &mem).unwrap();

            // legacy env 键被清除
            assert_eq!(mem.get("GEMINI_MODEL"), None);
            assert_eq!(
                mem.get("GEMINI_API_KEY").as_deref(),
                Some("sk-test")
            );

            // settings.json：modelProvider 打开 + model 写入
            let settings: Value = serde_json::from_str(
                &std::fs::read_to_string(get_antigravity_settings_path()).unwrap(),
            )
            .unwrap();
            assert_eq!(
                settings.get("modelProvider").and_then(Value::as_str),
                Some("gemini")
            );
            assert_eq!(
                settings.get("model").and_then(Value::as_str),
                Some("Gemini 3.8 Flash (Low)")
            );
            let _ = home;
        });
    }

    #[test]
    #[serial_test::serial]
    fn api_key_switch_without_model_keeps_existing_model() {
        with_test_home(|_home| {
            // agy 里已有用户自选的模型
            let settings_path = get_antigravity_settings_path();
            std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
            std::fs::write(
                &settings_path,
                r#"{"model":"Gemini 3.8 Flash (Low)"}"#,
            )
            .unwrap();

            let mem = MemEnv::new();
            let provider = provider_of(serde_json::json!({
                "authType": "api-key",
                "env": { "GEMINI_API_KEY": "sk-test" }
            }));
            write_antigravity_provider_live_with(&provider, &mem).unwrap();

            // 未配置默认模型 → agy 里的选择原样保留
            let settings: Value =
                serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
            assert_eq!(
                settings.get("model").and_then(Value::as_str),
                Some("Gemini 3.8 Flash (Low)")
            );
            assert_eq!(
                settings.get("modelProvider").and_then(Value::as_str),
                Some("gemini")
            );
        });
    }

    #[test]
    fn snapshot_carries_current_model() {
        let settings = serde_json::json!({
            "modelProvider": "gemini",
            "model": "Gemini 3.8 Flash (Low)"
        });
        let mut env = HashMap::new();
        env.insert("GEMINI_API_KEY".to_string(), "sk-test".to_string());
        let snapshot = compose_live_snapshot(&settings, &env, None);
        assert_eq!(snapshot.get("authType").and_then(Value::as_str), Some("api-key"));
        assert_eq!(
            snapshot.get("model").and_then(Value::as_str),
            Some("Gemini 3.8 Flash (Low)")
        );

        // 没写 model 的机器：快照不带该键（表单显示空 = 未配置）
        let settings_no_model = serde_json::json!({ "modelProvider": "gemini" });
        let snapshot = compose_live_snapshot(&settings_no_model, &env, None);
        assert!(snapshot.get("model").is_none());
    }

    #[test]
    #[serial_test::serial]
    fn oauth_switch_clears_legacy_model_env_but_keeps_settings_model() {
        with_test_home(|_home| {
            let settings_path = get_antigravity_settings_path();
            std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
            std::fs::write(
                &settings_path,
                r#"{"modelProvider":"gemini","model":"Gemini 3.8 Flash (Low)"}"#,
            )
            .unwrap();

            let mem = MemEnv::new();
            mem.set("GEMINI_MODEL", "legacy").unwrap();
            mem.set("GEMINI_API_KEY", "sk-old").unwrap();

            let provider = provider_of(serde_json::json!({
                "authType": "oauth",
                "token": { "access_token": "at", "refresh_token": "rt", "expiry": "2030-01-01T00:00:00Z" }
            }));
            write_antigravity_provider_live_with(&provider, &mem).unwrap();

            // 凭据面：env 全清（含 legacy），modelProvider 摘掉
            assert_eq!(mem.get("GEMINI_MODEL"), None);
            assert_eq!(mem.get("GEMINI_API_KEY"), None);
            let settings: Value =
                serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
            assert!(settings.get("modelProvider").is_none());
            // model 是 agy 全局偏好，不属于凭据面 → 不动
            assert_eq!(
                settings.get("model").and_then(Value::as_str),
                Some("Gemini 3.8 Flash (Low)")
            );
        });
    }
}
