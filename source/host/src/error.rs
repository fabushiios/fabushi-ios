use std::ffi::{c_char, CString};
use gpui_telegram_client::net::HttpClientError;
use gpui_telegram_core::error::TelegramError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse<T> {
    pub id: String,
    pub ok: bool,
    pub generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDetail>,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{code}: {message}")]
pub struct HostError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl HostError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }

    pub fn invalid_arguments(message: impl Into<String>) -> Self {
        Self::new("invalid_arguments", message, false)
    }

    pub fn invalid_json(message: impl Into<String>) -> Self {
        Self::new("invalid_json", message, false)
    }

    pub fn invalid_utf8(message: impl Into<String>) -> Self {
        Self::new("invalid_utf8", message, false)
    }

    pub fn unknown_command(method: &str) -> Self {
        Self::new(
            "unknown_command",
            format!("Unknown RPC method '{method}'"),
            false,
        )
    }

    pub fn stale_generation(requested: u64, current: u64) -> Self {
        Self::new(
            "stale_generation",
            format!(
                "Request generation {requested} is stale (current generation is {current})"
            ),
            false,
        )
    }

    pub fn capability_unavailable(message: impl Into<String>) -> Self {
        Self::new("capability_unavailable", message, false)
    }

    pub fn upstream_not_implemented(message: impl Into<String>) -> Self {
        Self::new("upstream_not_implemented", message, false)
    }

    pub fn cancelled(request_id: &str) -> Self {
        Self::new(
            "cancelled",
            format!("Request '{request_id}' was cancelled"),
            false,
        )
    }

    pub fn host_panic(message: impl Into<String>) -> Self {
        Self::new("host_panic", message, false)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new("unauthorized", message, false)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message, false)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new("conflict", message, false)
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self::new("network_error", message, true)
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::new("storage_error", message, false)
    }

    pub fn crypto(message: impl Into<String>) -> Self {
        Self::new("crypto_error", message, false)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal_error", message, false)
    }

    pub fn from_http(err: HttpClientError) -> Self {
        match err {
            HttpClientError::Unauthorized(msg) => Self::unauthorized(msg),
            HttpClientError::Conflict(msg) => Self::conflict(msg),
            HttpClientError::Proxy(msg) => Self::new("proxy_error", msg, true),
            HttpClientError::Serialization(e) => Self::invalid_json(e.to_string()),
            HttpClientError::Network(e) => Self::network(e.to_string()),
            HttpClientError::Api { status, message } => {
                let retryable = status == 429 || status >= 500;
                let code = match status {
                    400 | 422 => "invalid_arguments",
                    401 => "unauthorized",
                    403 => "permission_denied",
                    404 => "not_found",
                    409 => "conflict",
                    429 => "rate_limited",
                    501 => "upstream_not_implemented",
                    _ => "api_error",
                };
                Self::new(code, format!("HTTP {status}: {message}"), retryable)
            }
        }
    }

    pub fn from_telegram(err: TelegramError) -> Self {
        match err {
            TelegramError::Auth(msg) => Self::unauthorized(msg),
            TelegramError::PermissionDenied(msg) => Self::new("permission_denied", msg, false),
            TelegramError::NotFound(msg) => Self::not_found(msg),
            TelegramError::Validation(msg) => Self::invalid_arguments(msg),
            TelegramError::Network(msg) => Self::network(msg),
            TelegramError::Database(msg) => Self::storage(msg),
            TelegramError::Serialization(msg) => Self::invalid_json(msg),
            other => Self::internal(other.to_string()),
        }
    }
}

impl From<HttpClientError> for HostError {
    fn from(value: HttpClientError) -> Self {
        Self::from_http(value)
    }
}

impl From<TelegramError> for HostError {
    fn from(value: TelegramError) -> Self {
        Self::from_telegram(value)
    }
}

pub type HostResult<T = Value> = Result<T, HostError>;

pub fn make_success_json<T: Serialize>(id: &str, generation: u64, result: &T) -> String {
    let resp = JsonRpcResponse {
        id: id.to_string(),
        ok: true,
        generation,
        result: Some(result),
        error: None,
    };
    serde_json::to_string(&resp).unwrap_or_else(|_| {
        format!(
            "{{\"id\":{},\"ok\":false,\"generation\":{},\"error\":{{\"code\":\"serialization_error\",\"message\":\"Failed to serialize response\",\"retryable\":false}}}}",
            serde_json::to_string(id).unwrap_or_else(|_| "\"\"".to_string()),
            generation
        )
    })
}

pub fn make_error_json(id: &str, generation: u64, err: &HostError) -> String {
    let resp: JsonRpcResponse<Value> = JsonRpcResponse {
        id: id.to_string(),
        ok: false,
        generation,
        result: None,
        error: Some(ErrorDetail {
            code: err.code.clone(),
            message: err.message.clone(),
            retryable: err.retryable,
        }),
    };
    serde_json::to_string(&resp).unwrap_or_else(|_| {
        "{\"id\":\"\",\"ok\":false,\"generation\":0,\"error\":{\"code\":\"serialization_error\",\"message\":\"Error serialization failed\",\"retryable\":false}}".to_string()
    })
}

pub fn make_error_json_ptr(
    id: &str,
    generation: u64,
    code: &str,
    message: &str,
    retryable: bool,
) -> *mut c_char {
    let err = HostError::new(code, message, retryable);
    let json = make_error_json(id, generation, &err);
    CString::new(json)
        .unwrap_or_else(|_| {
            CString::new(
                "{\"id\":\"\",\"ok\":false,\"generation\":0,\"error\":{\"code\":\"serialization_error\",\"message\":\"NUL byte in error JSON\",\"retryable\":false}}",
            )
            .expect("static CString is valid")
        })
        .into_raw()
}
