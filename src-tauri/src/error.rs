use std::path::Path;
use std::sync::PoisonError;

/// 后端用户可见文案的语言选择：设置语言为 en 时走英文，其余（zh / zh-TW / ja）
/// 一律走中文——与 `services/provider/usage.rs` 的折叠口径一致（ja/zh-TW 回落 zh）。
/// 错误枚举（AppError / proxy::error 等）的 Display 都经由这里输出，测试环境
/// settings 未初始化时 language 为 None → 中文，既有断言不受影响。
pub fn prefer_en() -> bool {
    crate::settings::get_settings().language.as_deref() == Some("en")
}

/// 按当前 UI 语言在 zh / en 文案之间取一。
pub fn pick(zh: &str, en: &str) -> String {
    if prefer_en() {
        en.to_string()
    } else {
        zh.to_string()
    }
}

#[derive(Debug)]
pub enum AppError {
    Config(String),
    InvalidInput(String),
    /// Native files changed after OGG Switch last read them.
    Conflict(String),
    Io {
        path: String,
        source: std::io::Error,
    },
    IoContext {
        context: String,
        source: std::io::Error,
    },
    Json {
        path: String,
        source: serde_json::Error,
    },
    JsonSerialize {
        source: serde_json::Error,
    },
    Toml {
        path: String,
        source: toml::de::Error,
    },
    Lock(String),
    McpValidation(String),
    Message(String),
    HttpStatus {
        status: u16,
        body: String,
    },
    Localized {
        key: &'static str,
        zh: String,
        en: String,
    },
    Database(String),
    OmoConfigNotFound,
    AllProvidersCircuitOpen,
    NoProvidersConfigured,
}

impl AppError {
    pub fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    pub fn json(path: impl AsRef<Path>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    pub fn toml(path: impl AsRef<Path>, source: toml::de::Error) -> Self {
        Self::Toml {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    pub fn localized(key: &'static str, zh: impl Into<String>, en: impl Into<String>) -> Self {
        Self::Localized {
            key,
            zh: zh.into(),
            en: en.into(),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 手写 Display 以按设置语言输出（原先 thiserror 的中文静态文案对
        // en/ja 用户不友好）。zh 文案保持逐字不变，既有测试断言不受影响。
        let en = prefer_en();
        let s = match self {
            AppError::Config(d) => pick(&format!("配置错误: {d}"), &format!("Config error: {d}")),
            AppError::InvalidInput(d) => {
                pick(&format!("无效输入: {d}"), &format!("Invalid input: {d}"))
            }
            AppError::Conflict(d) => pick(&format!("并发冲突: {d}"), &format!("Conflict: {d}")),
            AppError::Io { path, source } => pick(
                &format!("IO 错误: {path}: {source}"),
                &format!("IO error: {path}: {source}"),
            ),
            AppError::IoContext { context, source } => pick(
                &format!("{context}: {source}"),
                &format!("{context}: {source}"),
            ),
            AppError::Json { path, source } => pick(
                &format!("JSON 解析错误: {path}: {source}"),
                &format!("JSON parse error: {path}: {source}"),
            ),
            AppError::JsonSerialize { source } => pick(
                &format!("JSON 序列化失败: {source}"),
                &format!("JSON serialization failed: {source}"),
            ),
            AppError::Toml { path, source } => pick(
                &format!("TOML 解析错误: {path}: {source}"),
                &format!("TOML parse error: {path}: {source}"),
            ),
            AppError::Lock(d) => pick(
                &format!("锁获取失败: {d}"),
                &format!("Lock acquisition failed: {d}"),
            ),
            AppError::McpValidation(d) => pick(
                &format!("MCP 校验失败: {d}"),
                &format!("MCP validation failed: {d}"),
            ),
            AppError::Message(d) => d.clone(),
            AppError::HttpStatus { status, body } => pick(
                &format!("HTTP {status}: {body}"),
                &format!("HTTP {status}: {body}"),
            ),
            AppError::Localized {
                zh, en: en_text, ..
            } => {
                if en {
                    en_text.clone()
                } else {
                    zh.clone()
                }
            }
            AppError::Database(d) => {
                pick(&format!("数据库错误: {d}"), &format!("Database error: {d}"))
            }
            AppError::OmoConfigNotFound => pick("OMO 配置文件不存在", "OMO config file not found"),
            AppError::AllProvidersCircuitOpen => pick(
                "所有供应商已熔断，无可用渠道",
                "All providers are circuit-broken; no channel available",
            ),
            AppError::NoProvidersConfigured => pick("未配置供应商", "No providers configured"),
        };
        write!(f, "{s}")
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppError::Io { source, .. } | AppError::IoContext { source, .. } => Some(source),
            AppError::Json { source, .. } => Some(source),
            AppError::JsonSerialize { source } => Some(source),
            AppError::Toml { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl<T> From<PoisonError<T>> for AppError {
    fn from(err: PoisonError<T>) -> Self {
        Self::Lock(err.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database(err.to_string())
    }
}

impl From<AppError> for String {
    fn from(err: AppError) -> Self {
        err.to_string()
    }
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// 格式化为 JSON 错误字符串，前端可解析为结构化错误
pub fn format_skill_error(
    code: &str,
    context: &[(&str, &str)],
    suggestion: Option<&str>,
) -> String {
    use serde_json::json;

    let mut ctx_map = serde_json::Map::new();
    for (key, value) in context {
        ctx_map.insert(key.to_string(), json!(value));
    }

    let error_obj = json!({
        "code": code,
        "context": ctx_map,
        "suggestion": suggestion,
    });

    serde_json::to_string(&error_obj).unwrap_or_else(|_| {
        // 如果 JSON 序列化失败，返回简单格式
        format!("ERROR:{code}")
    })
}
