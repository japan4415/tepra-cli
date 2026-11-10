use serde::Serialize;
use std::fmt;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Serialize)]
pub struct AppError {
    pub code: String,
    pub message: String,
    #[serde(skip)]
    pub exit_code: i32,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

impl AppError {
    pub fn new(code: &str, message: impl Into<String>, exit_code: i32) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            exit_code,
            retryable: false,
            context: None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_input", message, 2)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new("unsupported", message, 6)
    }

    pub fn unknown(message: impl Into<String>) -> Self {
        Self::new("print_result_unknown", message, 7)
    }

    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = Some(context);
        self
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}
