//! Shared user-facing error messages used across compiler and runtime.
//!
//! These constants are re-exported from semantic helpers so diagnostics, const-eval,
//! and runtime panics stay aligned.

pub use crate::strings::{STRING_INDEX_OUT_OF_RANGE_MSG, STRING_SLICE_STEP_ZERO_MSG};
