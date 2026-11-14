//! Error contract shared by every command, and the CLI exit-code mapping.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Usage,
    Validation,
    NotFound,
    DependencyMissing,
    Provider,
    Media,
    QcFailed,
    Cancelled,
    RevisionConflict,
    Internal,
}

impl ErrorCode {
    /// Process exit code for this error class (see Global Constraints).
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorCode::Usage | ErrorCode::Validation => 2,
            ErrorCode::NotFound => 3,
            ErrorCode::DependencyMissing => 4,
            ErrorCode::Provider => 5,
            ErrorCode::Media => 6,
            ErrorCode::QcFailed => 7,
            ErrorCode::Cancelled => 8,
            ErrorCode::RevisionConflict => 9,
            ErrorCode::Internal => 10,
        }
    }
}

#[derive(thiserror::Error, Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[error("{code:?}: {message}")]
pub struct RustlerError {
    pub code: ErrorCode,
    pub stage: String,
    pub message: String,
    pub hint: Option<String>,
    pub retryable: bool,
}

impl RustlerError {
    pub fn new(code: ErrorCode, stage: &str, message: impl Into<String>) -> Self {
        Self {
            code,
            stage: stage.to_string(),
            message: message.into(),
            hint: None,
            retryable: false,
        }
    }

    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }
}

pub type Result<T> = std::result::Result<T, RustlerError>;
