use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// 手写 Display 以按设置语言输出（zh 文案逐字保留，既有测试断言不受影响）。
impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ProxyError::ResponseBodyTooLarge(n) => crate::error::pick(
                &format!("上游响应体超过大小上限: {n} 字节"),
                &format!("Upstream response body exceeds the size limit: {n} bytes"),
            ),
            ProxyError::AlreadyRunning => {
                crate::error::pick("服务器已在运行", "Server is already running")
            }
            ProxyError::NotRunning => crate::error::pick("服务器未运行", "Server is not running"),
            ProxyError::BindFailed(d) => crate::error::pick(
                &format!("地址绑定失败: {d}"),
                &format!("Failed to bind address: {d}"),
            ),
            ProxyError::StopTimeout => crate::error::pick("停止超时", "Stop timed out"),
            ProxyError::StopFailed(d) => {
                crate::error::pick(&format!("停止失败: {d}"), &format!("Failed to stop: {d}"))
            }
            ProxyError::ForwardFailed(d) => crate::error::pick(
                &format!("请求转发失败: {d}"),
                &format!("Request forwarding failed: {d}"),
            ),
            ProxyError::NoAvailableProvider => {
                crate::error::pick("无可用的Provider", "No provider available")
            }
            ProxyError::AllProvidersCircuitOpen => crate::error::pick(
                "所有供应商已熔断，无可用渠道",
                "All providers are circuit-broken; no channel available",
            ),
            ProxyError::NoProvidersConfigured => {
                crate::error::pick("未配置供应商", "No providers configured")
            }
            ProxyError::ProviderUnhealthy(d) => crate::error::pick(
                &format!("Provider不健康: {d}"),
                &format!("Provider unhealthy: {d}"),
            ),
            ProxyError::UpstreamError { status, body } => crate::error::pick(
                &format!("上游错误 (状态码 {status}): {body:?}"),
                &format!("Upstream error (status {status}): {body:?}"),
            ),
            ProxyError::MaxRetriesExceeded => {
                crate::error::pick("超过最大重试次数", "Max retries exceeded")
            }
            ProxyError::DatabaseError(d) => {
                crate::error::pick(&format!("数据库错误: {d}"), &format!("Database error: {d}"))
            }
            ProxyError::ConfigError(d) => {
                crate::error::pick(&format!("配置错误: {d}"), &format!("Config error: {d}"))
            }
            ProxyError::TransformError(d) => crate::error::pick(
                &format!("格式转换错误: {d}"),
                &format!("Transform error: {d}"),
            ),
            ProxyError::InvalidRequest(d) => crate::error::pick(
                &format!("无效的请求: {d}"),
                &format!("Invalid request: {d}"),
            ),
            ProxyError::Timeout(d) => {
                crate::error::pick(&format!("超时: {d}"), &format!("Timeout: {d}"))
            }
            ProxyError::StreamIdleTimeout(secs) => crate::error::pick(
                &format!("流式响应空闲超时: {secs}秒无数据"),
                &format!("Streaming idle timeout: {secs}s without data"),
            ),
            ProxyError::AuthError(d) => crate::error::pick(
                &format!("认证失败: {d}"),
                &format!("Authentication failed: {d}"),
            ),
            ProxyError::Internal(d) => {
                crate::error::pick(&format!("内部错误: {d}"), &format!("Internal error: {d}"))
            }
        };
        write!(f, "{s}")
    }
}

#[derive(Debug)]
pub enum ProxyError {
    ResponseBodyTooLarge(usize),

    AlreadyRunning,

    NotRunning,

    BindFailed(String),

    StopTimeout,

    StopFailed(String),

    ForwardFailed(String),

    NoAvailableProvider,

    AllProvidersCircuitOpen,

    NoProvidersConfigured,

    #[allow(dead_code)]
    ProviderUnhealthy(String),

    UpstreamError {
        status: u16,
        body: Option<String>,
    },

    MaxRetriesExceeded,

    DatabaseError(String),

    ConfigError(String),

    #[allow(dead_code)]
    TransformError(String),

    #[allow(dead_code)]
    InvalidRequest(String),

    Timeout(String),

    /// 流式响应空闲超时
    #[allow(dead_code)]
    StreamIdleTimeout(u64),

    /// 认证错误
    AuthError(String),

    #[allow(dead_code)]
    Internal(String),
}

impl std::error::Error for ProxyError {}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let (status, body) = match &self {
            ProxyError::UpstreamError {
                status: upstream_status,
                body: upstream_body,
            } => {
                let http_status =
                    StatusCode::from_u16(*upstream_status).unwrap_or(StatusCode::BAD_GATEWAY);

                // 尝试解析上游响应体为 JSON，如果失败则包装为字符串
                let error_body = if let Some(body_str) = upstream_body {
                    if let Ok(json_body) = serde_json::from_str::<serde_json::Value>(body_str) {
                        // 上游返回的是 JSON，直接透传
                        json_body
                    } else {
                        // 上游返回的不是 JSON，包装为错误消息
                        json!({
                            "error": {
                                "message": body_str,
                                "type": "upstream_error",
                            }
                        })
                    }
                } else {
                    json!({
                        "error": {
                            "message": format!("Upstream error (status {})", upstream_status),
                            "type": "upstream_error",
                        }
                    })
                };

                (http_status, error_body)
            }
            _ => {
                let (http_status, message) = match &self {
                    ProxyError::AlreadyRunning => (StatusCode::CONFLICT, self.to_string()),
                    ProxyError::NotRunning => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
                    ProxyError::BindFailed(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::StopTimeout => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::StopFailed(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::ForwardFailed(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
                    ProxyError::NoAvailableProvider => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::AllProvidersCircuitOpen => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::NoProvidersConfigured => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::ProviderUnhealthy(_) => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::MaxRetriesExceeded => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::DatabaseError(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::ConfigError(_) => (StatusCode::BAD_REQUEST, self.to_string()),
                    ProxyError::TransformError(_) => {
                        (StatusCode::UNPROCESSABLE_ENTITY, self.to_string())
                    }
                    ProxyError::InvalidRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
                    ProxyError::Timeout(_) => (StatusCode::GATEWAY_TIMEOUT, self.to_string()),
                    ProxyError::StreamIdleTimeout(_) => {
                        (StatusCode::GATEWAY_TIMEOUT, self.to_string())
                    }
                    ProxyError::AuthError(_) => (StatusCode::UNAUTHORIZED, self.to_string()),
                    ProxyError::Internal(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::ResponseBodyTooLarge(_) => {
                        (StatusCode::BAD_GATEWAY, self.to_string())
                    }
                    ProxyError::UpstreamError { .. } => unreachable!(),
                };

                let error_body = json!({
                    "error": {
                        "message": message,
                        "type": "proxy_error",
                    }
                });

                (http_status, error_body)
            }
        };

        (status, Json(body)).into_response()
    }
}

/// 错误分类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// 可重试错误（网络问题、5xx）
    Retryable, // 网络超时、5xx 错误
    /// 不可重试错误（4xx、认证失败）
    NonRetryable, // 认证失败、参数错误、4xx 错误
    #[allow(dead_code)]
    ClientAbort, // 客户端主动中断
}

/// 判断错误是否可重试
#[allow(dead_code)]
pub fn categorize_error(error: &reqwest::Error) -> ErrorCategory {
    if error.is_timeout() || error.is_connect() {
        return ErrorCategory::Retryable;
    }

    if let Some(status) = error.status() {
        if status.is_server_error() {
            ErrorCategory::Retryable
        } else if status.is_client_error() {
            ErrorCategory::NonRetryable
        } else {
            ErrorCategory::Retryable
        }
    } else {
        ErrorCategory::Retryable
    }
}
