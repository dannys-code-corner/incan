//! Define error types for IR → Rust emission.
//!
//! These errors represent *backend emission* failures (as opposed to parsing or typechecking).
//!
//! ## Notes
//!
//! - Prefer actionable messages: users should know what construct is unsupported and what to do instead (e.g., compute
//!   at runtime).

/// Error during IR emission.
#[derive(Debug)]
pub enum EmitError {
    SynParse(String),
    Unsupported(String),
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmitError::SynParse(msg) => write!(f, "syn parse error: {}", msg),
            EmitError::Unsupported(msg) => write!(f, "unsupported: {}", msg),
        }
    }
}

impl std::error::Error for EmitError {}
