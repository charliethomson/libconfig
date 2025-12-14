use liberror::AnyError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Serialize, Deserialize, Clone, Error, valuable::Valuable)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "$type",
    content = "context"
)]
pub enum ConfigError {
    #[error("Failed to read config file at \"{path}\": {inner_error}")]
    Read { path: String, inner_error: AnyError },
    #[error("Failed to parse config: {inner_error}")]
    Parse { inner_error: AnyError },
    #[error("Failed to open file at \"{path}\": {inner_error}")]
    Open { path: String, inner_error: AnyError },
    #[error("Failed to dump config: {inner_error}")]
    Dump { inner_error: AnyError },
    #[error("Failed to write to config file at \"{path}\": {inner_error}")]
    Write { path: String, inner_error: AnyError },
}
